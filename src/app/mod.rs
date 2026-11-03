//! The windows over the game, like Hearthstone Deck Tracker's overlay.
//!
//! - A reader thread (`snapshot::run_live`) follows Power.log and sends a `Snapshot` with all the
//!   advice when something changes: the windows only draw it, so they never freeze.
//! - Left of the game: the minions of the lobby by tier (`left.rs`). Right of the game: the Comps and
//!   Lobby tabs (`right.rs`). At the top: the win chance of the fight (`odds`). Next to a panel: the full
//!   picture of the card under the mouse. Mouse over a hero portrait of the game's leaderboard: that
//!   player's last board seen (`board.rs`). The tray icon holds the menu (`tray.rs`), small windows the
//!   dialogs (`dialogs.rs`).
//! - The main window of the program is an invisible 1x1 "host": the panels are its child windows,
//!   shown or hidden twice a second like HDT (visible only while Hearthstone or Owlingstone is in front
//!   and a Battlegrounds game is on, unless auto-hide is off).
// The tray menu (Windows only) is what calls a few of these.
#![cfg_attr(not(windows), allow(dead_code))]

mod board;
mod dialogs;
mod draw;
mod images;
mod left;
mod right;
mod tray;
mod win;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Color32, Context, Pos2, ViewportBuilder, ViewportClass, ViewportId, pos2, vec2};
use owlingstone::history::{self, Record};
use owlingstone::settings::Settings;
use owlingstone::share::Share;
use owlingstone::snapshot::{self, Snapshot};
use owlingstone::update::{self, Updater};
use owlingstone::{VERSION, cards, data, hslogs};

use draw::Hover;
use images::Images;

/// Width of the right panel (before the player's narrower / wider setting).
pub const HUD_WIDTH: f32 = 300.0;
pub const PANEL_MIN_WIDTH: f32 = 170.0;
pub const PANEL_STEP: i64 = 20;
pub const ODDS_W: f32 = 300.0;
pub const ODDS_H: f32 = 44.0 + draw::ODDS_LABEL_H;
const PREVIEW_H: f32 = 330.0;
/// Panels are slightly see-through, like the Python version's 0.93 opacity.
const ALPHA: u8 = 238;

/// Where a side panel goes: in the decor band around the game's 4:3 area (the hero leaderboard is just
/// inside that area on the left, the board in the middle). `delta`: the player's narrower / wider setting.
pub fn side_panel_geometry(rect: (f32, f32, f32, f32), left: bool, delta: f32) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = rect;
    let free = (w - h * 4.0 / 3.0) / 2.0;
    let width = if left { 320f32.min(free * 0.8) } else { HUD_WIDTH.min(free - 10.0) };
    let width = (width + delta).max(PANEL_MIN_WIDTH).round();
    let x = if left { x } else { x + w - width };
    (x, y + (h * 0.06).round(), width, (h * 0.85).round())
}

/// Centred at the top of the game, just above the opponent's portrait.
pub fn odds_geometry(rect: (f32, f32, f32, f32)) -> (f32, f32) {
    let (x, y, w, h) = rect;
    ((x + (w - ODDS_W) / 2.0).round(), (y + h * 0.005).round())
}

/// What the update checker says.
enum UpdateMsg {
    Ready(String),
}

/// A short message near the bottom right of the screen (what Qt showed as a tray balloon).
pub struct Toast {
    pub title: String,
    pub text: String,
    pub until: Instant,
}

/// How the app was started.
pub struct Options {
    /// Re-read an old log up to this turn instead of following the live game (window always shown).
    pub replay: Option<(PathBuf, Option<i64>)>,
    /// Test of the built exe: opens, then closes by itself.
    pub smoke: bool,
}

pub struct App {
    rx: Receiver<Arc<Snapshot>>,
    stop: Arc<AtomicBool>,
    pub snap: Arc<Snapshot>,
    pub settings: Settings,
    pub images: Images,
    // --- state of the panels
    /// None = the list follows my tavern tier; else the tier clicked.
    pub manual_tier: Option<i64>,
    /// A tier 7 card appeared in this game.
    pub seen_tier7: bool,
    pub keyword: Option<&'static str>,
    /// Comp opened on the right: None = the one I play, Some(None) = none, Some(Some(id)) = that one.
    pub open_comp: Option<Option<String>>,
    /// PLAYER_ID clicked in the Lobby tab (None = my next opponent).
    pub selected_player: Option<i64>,
    pub folded: BTreeSet<String>,
    pub tab: String,
    /// The card under the mouse this frame, and the screen rect of its window.
    hover: Option<(Hover, egui::Rect)>,
    /// The cursor over the game's leaderboard.
    board_watch: board::Watch,
    // --- placement
    game_rect: Option<win::Rect>,
    visible: bool,
    last_place: Option<Instant>,
    /// Set by the watcher thread as soon as the window in front changes: show / hide right away.
    front_changed: Arc<AtomicBool>,
    host_pos: Pos2,
    other_tracker: bool,
    replay: bool,
    smoke_until: Option<Instant>,
    // --- updates, dialogs, messages
    updater: Arc<Mutex<Updater>>,
    upd_rx: Receiver<UpdateMsg>,
    pub update_ready: Option<String>,
    pub dialogs: dialogs::Dialogs,
    pub toasts: Vec<Toast>,
    tray: Option<tray::Tray>,
    /// The tray menu, while it is open (tray.rs).
    pub tray_menu: Option<tray::OpenMenu>,
    asked_sharing: bool,
    history_cache: (Option<SystemTime>, Arc<Vec<Record>>),
}

/// Starts the window. Returns the exit code.
pub fn run(opts: Options) -> i32 {
    let replay = opts.replay.is_some();
    let after_update = std::env::var_os(update::AFTER_UPDATE).is_some_and(|v| v == "1");
    // SAFETY: called on the main thread before any other thread reads the environment.
    unsafe { std::env::remove_var(update::AFTER_UPDATE) };
    // One Owlingstone at a time; right after an update the old one is still closing: wait for it.
    #[cfg_attr(not(windows), allow(unused_variables))]
    let instance = if replay {
        None
    } else {
        match win::single_instance(if after_update { Duration::from_secs(20) } else { Duration::from_millis(100) }) {
            Some(i) => Some(i),
            None => return 0,
        }
    };
    update::remove_old_exe();
    let native = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("Owlingstone")
            .with_inner_size([1.0, 1.0])
            .with_position([0.0, 0.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_taskbar(false)
            .with_mouse_passthrough(true)
            .with_resizable(false)
            .with_icon(icon()),
        renderer: renderer(),
        ..Default::default()
    };
    #[cfg(windows)]
    let opts_smoke = opts.smoke;
    let result = eframe::run_native(
        "Owlingstone",
        native,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            setup_style(&cc.egui_ctx);
            Ok(Box::new(App::new(&cc.egui_ctx, opts)))
        }),
    );
    match result {
        Ok(()) => 0,
        Err(e) => {
            owlingstone::errlog::note("window", &e.to_string());
            // No OpenGL 2 (no graphics driver, virtual machine, remote desktop): Windows can still draw
            // with Direct3D 12, in software if needed. A window can be opened only once per process,
            // so the fallback runs in a new one.
            #[cfg(windows)]
            if !dx12_fallback() {
                drop(instance);
                if let Some(code) = run_with_dx12() {
                    return code;
                }
            }
            eprintln!("Owlingstone could not open its window: {e}");
            #[cfg(windows)]
            if !opts_smoke {
                rfd::MessageDialog::new()
                    .set_title("Owlingstone")
                    .set_level(rfd::MessageLevel::Error)
                    .set_description(format!(
                        "Owlingstone could not open its window ({e}).\n\nUpdating your graphics driver usually fixes this."
                    ))
                    .show();
            }
            1
        }
    }
}

/// Set when this process is the Direct3D fallback started by `run_with_dx12`.
const RENDERER_ENV: &str = "OWLINGSTONE_RENDERER";

fn dx12_fallback() -> bool {
    std::env::var_os(RENDERER_ENV).is_some_and(|v| v == "dx12")
}

#[cfg(windows)]
fn renderer() -> eframe::Renderer {
    if dx12_fallback() { eframe::Renderer::Wgpu } else { eframe::Renderer::Glow }
}

#[cfg(not(windows))]
fn renderer() -> eframe::Renderer {
    eframe::Renderer::Glow
}

/// Starts this exe again with the same arguments, drawing with Direct3D 12; its exit code.
#[cfg(windows)]
fn run_with_dx12() -> Option<i32> {
    let exe = std::env::current_exe().ok()?;
    let status = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(RENDERER_ENV, "dx12")
        .env(update::AFTER_UPDATE, "1") // wait for this process to let go of the one-instance lock
        .status()
        .ok()?;
    Some(status.code().unwrap_or(1))
}

fn icon() -> egui::IconData {
    let img = image::load_from_memory_with_format(include_bytes!("../../assets/icon.png"), image::ImageFormat::Png)
        .map(|i| i.to_rgba8())
        .ok();
    match img {
        Some(i) => egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() },
        None => egui::IconData::default(),
    }
}

/// Segoe UI (and its symbol / emoji fonts) from Windows when there: the look of the Python version.
/// egui's own fonts stay as fallback (other systems, missing glyphs).
fn setup_fonts(ctx: &Context) {
    let mut fonts = egui::FontDefinitions::default();
    let dir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("Fonts");
    let mut load = |name: &str, file: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(bytes) => {
                fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    };
    let regular = load("segoe", "segoeui.ttf");
    let strong = load("segoe-bold", "segoeuib.ttf");
    let symbols = load("segoe-symbol", "seguisym.ttf");
    let emoji = load("segoe-emoji", "seguiemj.ttf");
    let prop = fonts.families.entry(egui::FontFamily::Proportional).or_default();
    let mut front = Vec::new();
    if regular {
        front.push("segoe".to_string());
    }
    if symbols {
        front.push("segoe-symbol".to_string());
    }
    if emoji {
        front.push("segoe-emoji".to_string());
    }
    let base = prop.clone();
    *prop = front.iter().cloned().chain(base.iter().cloned()).collect();
    let mut bold: Vec<String> = Vec::new();
    if strong {
        bold.push("segoe-bold".into());
    }
    bold.extend(prop.iter().cloned());
    fonts.families.insert(egui::FontFamily::Name("bold".into()), bold);
    ctx.set_fonts(fonts);
}

fn setup_style(ctx: &Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.global_style_mut(|s| {
        s.visuals.panel_fill = Color32::from_rgba_unmultiplied(0x1b, 0x1d, 0x23, ALPHA);
        s.visuals.window_fill = draw::PANEL;
        s.visuals.override_text_color = Some(draw::TEXT);
        s.visuals.widgets.inactive.weak_bg_fill = draw::BUTTON;
        s.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, draw::BORDER);
        s.visuals.selection.bg_fill = draw::GOLD;
        s.visuals.selection.stroke = egui::Stroke::new(1.0, Color32::from_rgb(0x11, 0x11, 0x11));
        s.spacing.item_spacing = vec2(4.0, 3.0);
        s.text_styles.insert(egui::TextStyle::Body, draw::font(12.0));
        s.text_styles.insert(egui::TextStyle::Button, draw::font(12.0));
        s.text_styles.insert(egui::TextStyle::Small, draw::font(10.0));
        s.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::monospace(12.0));
        s.interaction.tooltip_delay = 0.3;
    });
}

impl App {
    fn new(ctx: &Context, opts: Options) -> App {
        owlingstone::errlog::install_panic_hook();
        let settings = Settings::load();
        let (tx, rx) = channel::<Arc<Snapshot>>();
        let stop = Arc::new(AtomicBool::new(false));
        let replay = opts.replay.is_some();
        // The log reader: a replayed snapshot once, or the live game.
        {
            let ctx = ctx.clone();
            let stop = stop.clone();
            let sink: snapshot::Sink = Box::new(move |s| {
                let _ = tx.send(s);
                ctx.request_repaint();
            });
            let replay = opts.replay.clone();
            let _ = std::thread::Builder::new().name("log reader".into()).spawn(move || match replay {
                Some((path, turn)) => match snapshot::replay_snapshot(&path, turn) {
                    Ok(s) => sink(Arc::new(s)),
                    Err(e) => eprintln!("{}: {e}", path.display()),
                },
                None => snapshot::run_live(sink, stop),
            });
        }
        let (upd_tx, upd_rx) = channel::<UpdateMsg>();
        let updater = Arc::new(Mutex::new(Updater::new(VERSION)));
        let mut app = App {
            rx,
            stop,
            snap: Arc::new(Snapshot { data_errors: owlingstone::knowledge::get().errors.clone(), ..Default::default() }),
            images: Images::new(ctx),
            manual_tier: None,
            seen_tier7: false,
            keyword: None,
            open_comp: None,
            selected_player: None,
            folded: settings.folded(),
            tab: settings.hud_tab(),
            settings,
            hover: None,
            board_watch: board::Watch::start(ctx),
            game_rect: None,
            visible: replay,
            last_place: None,
            front_changed: Arc::new(AtomicBool::new(false)),
            host_pos: pos2(0.0, 0.0),
            other_tracker: false,
            replay,
            smoke_until: opts.smoke.then(|| Instant::now() + Duration::from_secs(4)),
            updater: updater.clone(),
            upd_rx,
            update_ready: None,
            dialogs: dialogs::Dialogs::default(),
            toasts: Vec::new(),
            tray: None,
            tray_menu: None,
            asked_sharing: false,
            history_cache: (None, Arc::new(Vec::new())),
        };
        if replay || opts.smoke {
            return app;
        }
        app.tray = tray::Tray::new(&app, ctx);
        // Watches which window is in front (a cheap check 20 times a second), so the panels show or hide
        // right when the player switches to or away from Hearthstone instead of at the next placement.
        {
            let (ctx, stop, flag) = (ctx.clone(), app.stop.clone(), app.front_changed.clone());
            let _ = std::thread::Builder::new().name("front window".into()).spawn(move || {
                let mut last = win::foreground_window();
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                    let now = win::foreground_window();
                    if now != last {
                        last = now;
                        flag.store(true, Ordering::Relaxed);
                        ctx.request_repaint();
                    }
                }
            });
        }
        // Other trackers show their own win chance at the same place: ours is off by default then.
        app.other_tracker = win::other_tracker_running();
        cards::refresh_in_background(false);
        // Updates: checked in the background (every 6 hours at most, see update.rs).
        {
            let ctx = ctx.clone();
            let _ = std::thread::Builder::new().name("updates".into()).spawn(move || {
                loop {
                    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                    let found = updater.lock().unwrap_or_else(|e| e.into_inner()).check_once(&update::github_get, now);
                    match found {
                        Ok(Some(info)) => {
                            let _ = upd_tx.send(UpdateMsg::Ready(info.version));
                            ctx.request_repaint();
                            return;
                        }
                        Ok(None) => {}
                        Err(e) => owlingstone::errlog::note("update check", &e),
                    }
                    std::thread::sleep(Duration::from_secs(600));
                }
            });
        }
        app.announce_version();
        let share = Share::default();
        if !share.asked() && !app.history().is_empty() {
            app.dialogs.ask_sharing = true; // already some games: a good time to ask
            app.asked_sharing = true;
        }
        if hslogs::ensure_log_config(None) == Some(hslogs::LogConfig::Written) {
            app.toast("Owlingstone", "Hearthstone logging is now on. If the game is open, restart it once so Owlingstone can read it.");
        }
        snapshot::send_in_background(share); // feedback or games left in the queue
        let _ = std::thread::Builder::new().name("stats".into()).spawn(|| {
            Share::default().download_stats();
        });
        app
    }

    pub fn save_settings(&mut self) {
        self.settings.set_folded(&self.folded);
        self.settings.set_text("hud_tab", Some(&self.tab.clone()));
        if let Err(e) = self.settings.save() {
            owlingstone::errlog::note("settings.json", &e.to_string());
        }
    }

    pub fn toast(&mut self, title: &str, text: &str) {
        self.toasts.push(Toast { title: title.into(), text: text.into(), until: Instant::now() + Duration::from_secs(10) });
    }

    /// First start of a new version: what changed (once).
    fn announce_version(&mut self) {
        if self.settings.last_version().as_deref() == Some(VERSION) {
            return;
        }
        if self.settings.last_version().is_some() {
            self.show_whats_new();
        }
        self.settings.set_text("last_version", Some(VERSION));
        self.save_settings();
    }

    pub fn show_whats_new(&mut self) {
        let lines = data::whats_new(VERSION);
        let text = if lines.is_empty() {
            "No notes for this version.".to_string()
        } else {
            lines.iter().take(4).map(|l| format!("• {l}")).collect::<Vec<_>>().join("\n")
        };
        self.toast(&format!("Owlingstone v{VERSION}"), &text);
    }

    /// My games (read again only when the file changed).
    pub fn history(&mut self) -> Arc<Vec<Record>> {
        let path = history::history_path();
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if stamp != self.history_cache.0 {
            self.history_cache = (stamp, Arc::new(history::load_history(&path)));
        }
        self.history_cache.1.clone()
    }

    pub fn top_odds(&self) -> bool {
        self.settings.top_odds(!self.other_tracker)
    }

    pub fn resize_panels(&mut self, step: i64) {
        let d = (self.settings.panel_delta() + step).clamp(-owlingstone::settings::PANEL_DELTA_MAX, owlingstone::settings::PANEL_DELTA_MAX);
        self.settings.set_number("panel_delta", d);
        self.save_settings();
    }

    /// The comp opened in the right panel (the one clicked, else the one I play).
    pub fn shown_comp(&self) -> Option<String> {
        match &self.open_comp {
            None => self.snap.advice.playing.as_ref().map(|p| p.comp_id.clone()),
            Some(c) => c.clone(),
        }
    }

    pub fn run_update(&mut self) {
        let result = self.updater.lock().unwrap_or_else(|e| e.into_inner()).install();
        match result {
            Ok(()) => self.quit(),
            Err(e) => {
                owlingstone::errlog::note("update", &e);
                self.dialogs.message = Some(("Owlingstone update".into(), format!("The update could not be installed; this version keeps working.\n\n{e}")));
            }
        }
    }

    pub fn quit(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.dialogs.quit = true;
    }

    fn on_snapshot(&mut self, snap: Arc<Snapshot>) {
        if snap.tier != self.snap.tier {
            self.manual_tier = None; // the tier went up: the list follows again
        }
        let new_game = !snap.in_game || snap.turn < self.snap.turn;
        if new_game {
            self.seen_tier7 = false;
            self.selected_player = None;
            self.open_comp = None;
        }
        let db = cards::db();
        self.seen_tier7 |= snap.shop.iter().chain(&snap.board).chain(&snap.hand).any(|id| db.get(id).is_some_and(|c| c.tier == Some(7)));
        // First finished game: the right moment to ask about sharing.
        if snap.over && !self.replay && !self.asked_sharing && !Share::default().asked() {
            self.asked_sharing = true;
            self.dialogs.ask_sharing = true;
        }
        self.snap = snap;
    }

    /// Twice a second, and right away when the window in front changes: stick to Hearthstone, and show /
    /// hide like HDT.
    fn place(&mut self, ctx: &Context) {
        let front_changed = self.front_changed.swap(false, Ordering::Relaxed);
        if !front_changed && self.last_place.is_some_and(|t| t.elapsed() < Duration::from_millis(500)) {
            return;
        }
        self.last_place = Some(Instant::now());
        // Windows fades windows in and out; new windows (dialogs, menu) get the same treatment here.
        win::no_fade_on_own_windows();
        if self.replay {
            self.visible = true;
            return;
        }
        self.game_rect = win::hearthstone_rect();
        *self.board_watch.game.lock().unwrap_or_else(|e| e.into_inner()) = self.game_rect;
        if !self.settings.hide_when_unfocused() {
            self.visible = true;
        } else {
            let exe = win::foreground_exe();
            let focused = exe == "hearthstone.exe" || exe == win::own_exe();
            self.visible = focused && self.snap.in_game && self.game_rect.is_some();
        }
        // The host sits inside the game's corner, so its scale is the one of the game's screen.
        if let Some((x, y, _, _)) = self.game_rect {
            let ppp = self.ppp(ctx);
            let p = pos2(x as f32 / ppp, y as f32 / ppp);
            if p != self.host_pos {
                self.host_pos = p;
                ctx.send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::OuterPosition(p));
            }
        }
    }

    fn ppp(&self, ctx: &Context) -> f32 {
        ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(1.0).max(0.5)
    }

    /// The game's area in points.
    fn game_points(&self, ctx: &Context) -> Option<(f32, f32, f32, f32)> {
        let ppp = self.ppp(ctx);
        self.game_rect.map(|(x, y, w, h)| (x as f32 / ppp, y as f32 / ppp, w as f32 / ppp, h as f32 / ppp))
    }

    fn panel_builder(title: &str, pos: (f32, f32), size: (f32, f32), visible: bool) -> ViewportBuilder {
        ViewportBuilder::default()
            .with_title(title)
            .with_position([pos.0, pos.1])
            .with_inner_size([size.0, size.1])
            .with_decorations(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_transparent(true)
            .with_visible(visible)
            .with_active(false)
    }

    fn show_panels(&mut self, ctx: &Context) {
        let delta = self.settings.panel_delta() as f32;
        let game = self.game_points(ctx);
        let (left_geo, right_geo) = match game {
            Some(g) if !self.replay => (side_panel_geometry(g, true, delta), Some(side_panel_geometry(g, false, delta))),
            _ => ((40.0, 40.0, 340.0, 760.0), None),
        };
        let right_geo = right_geo.unwrap_or((left_geo.0 + left_geo.2 + 10.0, left_geo.1, HUD_WIDTH + delta.max(0.0), left_geo.3));
        let visible = self.visible;
        let mut hover: Option<(Hover, egui::Rect)> = None;
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("left"),
            Self::panel_builder("Owlingstone", (left_geo.0, left_geo.1), (left_geo.2, left_geo.3), visible),
            |ui, class| {
                let ctx = &ui.ctx().clone();
                if class == ViewportClass::EmbeddedWindow {
                    return;
                }
                let mut h = None;
                left::show(self, ui, &mut h);
                if let Some(h) = h {
                    hover = Some((h, ctx.input(|i| i.viewport().outer_rect).unwrap_or(egui::Rect::NOTHING)));
                }
            },
        );
        let right_on = visible && self.settings.lobby_panel();
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("right"),
            Self::panel_builder("Owlingstone – Lobby", (right_geo.0, right_geo.1), (right_geo.2, right_geo.3), right_on),
            |ui, class| {
                let ctx = &ui.ctx().clone();
                if class == ViewportClass::EmbeddedWindow {
                    return;
                }
                let mut h = None;
                right::show(self, ui, &mut h);
                if let Some(h) = h {
                    hover = Some((h, ctx.input(|i| i.viewport().outer_rect).unwrap_or(egui::Rect::NOTHING)));
                }
            },
        );
        // Win chance at the top of the game (clicks go through it).
        let odds = self.snap.combat.as_ref().and_then(|c| c.now.clone());
        let odds_on = visible && self.snap.in_game && odds.is_some() && self.top_odds() && (game.is_some() || self.replay);
        let pos = match game {
            Some(g) if !self.replay => odds_geometry(g),
            _ => (left_geo.0 + left_geo.2 + 10.0, 4.0),
        };
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("odds"),
            ViewportBuilder::default()
                .with_title("Owlingstone – win chance")
                .with_position([pos.0, pos.1])
                .with_inner_size([ODDS_W, ODDS_H])
                .with_decorations(false)
                .with_always_on_top()
                .with_taskbar(false)
                .with_resizable(false)
                .with_mouse_passthrough(true)
                .with_visible(odds_on)
                .with_active(false),
            |ui, class| {
                if class == ViewportClass::EmbeddedWindow {
                    return;
                }
                right::odds_banner(ui, odds.as_ref());
            },
        );
        self.show_preview(ctx, hover);
        self.show_board(ctx, game);
    }

    /// The last board of the player whose portrait is under the mouse (left of the game's board).
    fn show_board(&mut self, ctx: &Context, game: Option<(f32, f32, f32, f32)>) {
        let active = self.visible && self.snap.in_game && !self.snap.over && !self.snap.duos && !self.replay && self.settings.board_on_hover();
        self.board_watch.active.store(active, Ordering::Relaxed);
        let snap = self.snap.clone();
        let shown = match (active, game, self.board_watch.place()) {
            (true, Some(g), Some(place)) => board::player_at(&snap, place).map(|p| (p, board::board_geometry(g, place, board::card_count(p)))),
            _ => None,
        };
        let ((x, y, w, h), card_w) = shown.as_ref().map(|s| s.1).unwrap_or(((0.0, 0.0, 10.0, 10.0), 0.0));
        let art = self.settings.card_art();
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("board"),
            ViewportBuilder::default()
                .with_title("Owlingstone – board")
                .with_position([x, y])
                .with_inner_size([w, h])
                .with_decorations(false)
                .with_always_on_top()
                .with_taskbar(false)
                .with_resizable(false)
                .with_transparent(true)
                .with_mouse_passthrough(true)
                .with_visible(shown.is_some())
                .with_active(false),
            |ui, class| {
                if class == ViewportClass::EmbeddedWindow {
                    return;
                }
                match &shown {
                    Some((p, _)) => board::show(ui, p, card_w, &mut self.images, art),
                    None => {
                        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |_| {});
                    }
                }
            },
        );
    }

    /// Full picture of the card under the mouse, beside its window, on the side of the screen's centre.
    fn show_preview(&mut self, ctx: &Context, hover: Option<(Hover, egui::Rect)>) {
        let wanted = hover.filter(|_| self.visible);
        let tex = wanted.as_ref().and_then(|(h, _)| self.images.card(&h.card_id));
        let (pos, size, on) = match (&wanted, &tex) {
            (Some((h, win)), Some(t)) => {
                let [w, hh] = t.size();
                let scale = PREVIEW_H / hh as f32;
                let size = (w as f32 * scale, PREVIEW_H);
                let screen_w = ctx.input(|i| i.viewport().monitor_size).map(|s| s.x).unwrap_or(1920.0);
                let x = if win.center().x > screen_w / 2.0 { win.left() - size.0 - 6.0 } else { win.right() + 6.0 };
                let y = (h.y + win.top() - size.1 / 2.0).max(0.0);
                ((x, y), size, true)
            }
            _ => ((0.0, 0.0), (10.0, 10.0), false),
        };
        self.hover = wanted.clone();
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("preview"),
            ViewportBuilder::default()
                .with_title("Owlingstone – card")
                .with_position([pos.0, pos.1])
                .with_inner_size([size.0, size.1])
                .with_decorations(false)
                .with_always_on_top()
                .with_taskbar(false)
                .with_resizable(false)
                .with_transparent(true)
                .with_mouse_passthrough(true)
                .with_visible(on)
                .with_active(false),
            |ui, class| {
                if class == ViewportClass::EmbeddedWindow {
                    return;
                }
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                    if let Some(t) = &tex {
                        ui.add(egui::Image::new(t).fit_to_exact_size(ui.available_size()));
                    }
                });
            },
        );
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        while let Ok(s) = self.rx.try_recv() {
            self.on_snapshot(s);
        }
        while let Ok(msg) = self.upd_rx.try_recv() {
            match msg {
                UpdateMsg::Ready(v) => {
                    if self.update_ready.as_deref() != Some(&v) {
                        self.toast("Owlingstone", &format!("Version {v} is ready. Click \"Update to v{v}\" in the window or the tray menu."));
                        self.update_ready = Some(v);
                    }
                }
            }
        }
        self.images.receive(ctx);
        if let Some(mut t) = self.tray.take() {
            let ppp = self.ppp(ctx);
            t.poll(self, ppp);
            self.tray = Some(t);
        }
        self.place(ctx);
        // The host itself shows nothing.
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |_| {});
        self.show_panels(ctx);
        dialogs::show(self, ctx);
        tray::show_menu(self, ctx);
        if self.dialogs.quit || self.smoke_until.is_some_and(|t| Instant::now() >= t) {
            self.stop.store(true, Ordering::Relaxed);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop.store(true, Ordering::Relaxed);
        self.save_settings();
    }
}

/// The folder Hearthstone is in, as the player chose it (tray menu).
pub fn choose_hearthstone_dir(app: &mut App) {
    #[cfg(windows)]
    {
        let start = app.settings.hearthstone_dir().map(PathBuf::from).or_else(hslogs::find_logs_dir);
        let mut dialog = rfd::FileDialog::new().set_title("Hearthstone folder (the one with Hearthstone.exe)");
        if let Some(s) = start {
            dialog = dialog.set_directory(s);
        }
        if let Some(folder) = dialog.pick_folder() {
            app.settings.set_text("hearthstone_dir", Some(&folder.to_string_lossy()));
            app.save_settings();
        }
    }
    #[cfg(not(windows))]
    {
        let _ = app;
    }
}

pub fn open_data_folder() {
    win::open_folder(&owlingstone::paths::user_dir());
}
