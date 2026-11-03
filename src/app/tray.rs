//! Tray icon near the clock (like HDT): the panels have no frame nor taskbar button, so the menu is
//! here: show the window, my stats, win chance at the top, panel width, what's new, Hearthstone folder,
//! data folder, update, feedback, sharing, quit.
//!
//! The menu is drawn by the app itself (`show_menu`), in the gold-on-dark look of the panels, instead of
//! the white Windows menu: a click on the icon (Windows only, `Tray`) opens a small frameless window just
//! above the mouse. It closes when an item is clicked, on Escape, or when another window is clicked.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Context, CornerRadius, Pos2, Sense, Stroke, ViewportBuilder, ViewportClass, ViewportId, pos2, vec2};

use super::{App, draw};

/// Background of the item under the mouse: a dark gold.
const HOVER: Color32 = Color32::from_rgb(0x3d, 0x33, 0x14);
const WIDTH: f32 = 270.0;
const ITEM_H: f32 = 24.0;
const SEP_H: f32 = 9.0;
const HEADER_H: f32 = 30.0;
const PAD: f32 = 6.0;
/// Closed anyway after this long without focus nor mouse over it (if Windows never gave it the focus).
const STALE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Show,
    Stats,
    Odds,
    Boards,
    Narrower,
    Wider,
    News,
    HsDir,
    Folder,
    Update,
    Feedback,
    Sharing,
    Sent,
    Erase,
    Quit,
}

enum Entry {
    Item { action: Action, label: String, enabled: bool, checked: Option<bool> },
    Separator,
}

/// The open menu: where (screen points, bottom left corner at the mouse) and since when.
pub struct OpenMenu {
    at: Pos2,
    opened: Instant,
    focused: bool,
    last_hover: Instant,
}

impl OpenMenu {
    pub fn new(at: Pos2) -> OpenMenu {
        let now = Instant::now();
        OpenMenu { at, opened: now, focused: false, last_hover: now }
    }
}

fn entries(app: &App) -> Vec<Entry> {
    let item = |action, label: &str| Entry::Item { action, label: label.to_string(), enabled: true, checked: None };
    let sharing = owlingstone::share::Share::default().enabled();
    let update = match &app.update_ready {
        Some(v) => Entry::Item { action: Action::Update, label: format!("Update to v{v}"), enabled: true, checked: None },
        None => Entry::Item { action: Action::Update, label: "No update ready".into(), enabled: false, checked: None },
    };
    vec![
        item(Action::Show, "Show window (turns off auto-hide)"),
        item(Action::Stats, "My stats"),
        Entry::Item { action: Action::Odds, label: "Win chance at the top".into(), enabled: true, checked: Some(app.top_odds()) },
        Entry::Item {
            action: Action::Boards,
            label: "Opponent board on portrait hover".into(),
            enabled: true,
            checked: Some(app.settings.board_on_hover()),
        },
        item(Action::Narrower, "Side panels narrower"),
        item(Action::Wider, "Side panels wider"),
        item(Action::News, "What's new"),
        item(Action::HsDir, "Choose Hearthstone folder…"),
        item(Action::Folder, "Open my data folder"),
        update,
        Entry::Separator,
        item(Action::Feedback, "Send feedback…"),
        Entry::Item { action: Action::Sharing, label: "Share my games (anonymous)".into(), enabled: true, checked: Some(sharing) },
        item(Action::Sent, "Show what is sent"),
        item(Action::Erase, "Delete my shared games"),
        Entry::Separator,
        item(Action::Quit, "Quit Owlingstone"),
    ]
}

/// Height of the menu window for these entries.
fn height(entries: &[Entry]) -> f32 {
    let body: f32 = entries.iter().map(|e| if matches!(e, Entry::Separator) { SEP_H } else { ITEM_H }).sum();
    HEADER_H + body + 2.0 * PAD
}

/// Top left corner of the menu: its bottom left at the mouse, kept on the screen.
pub fn menu_origin(at: Pos2, size: egui::Vec2, screen: egui::Vec2) -> Pos2 {
    let x = at.x.min(screen.x - size.x - 4.0).max(0.0);
    let y = (at.y - size.y).max(0.0);
    pos2(x, y)
}

pub fn apply(app: &mut App, action: Action) {
    match action {
        Action::Show => {
            app.settings.set_flag("hide_when_unfocused", false);
            app.save_settings();
        }
        Action::Stats => app.dialogs.stats = Some(String::new()),
        Action::Odds => {
            let on = !app.top_odds();
            app.settings.set_flag("top_odds", on);
            app.save_settings();
        }
        Action::Boards => {
            let on = !app.settings.board_on_hover();
            app.settings.set_flag("board_on_hover", on);
            app.save_settings();
        }
        Action::Narrower => app.resize_panels(-super::PANEL_STEP),
        Action::Wider => app.resize_panels(super::PANEL_STEP),
        Action::News => app.show_whats_new(),
        Action::HsDir => super::choose_hearthstone_dir(app),
        Action::Folder => super::open_data_folder(),
        Action::Update => app.run_update(),
        Action::Feedback => app.dialogs.feedback = Some(super::dialogs::new_feedback()),
        Action::Sharing => {
            let share = owlingstone::share::Share::default();
            if let Err(e) = share.set_enabled(!share.enabled()) {
                owlingstone::errlog::note("sharing.json", &e.to_string());
            }
        }
        Action::Sent => app.dialogs.sent = true,
        Action::Erase => app.dialogs.confirm_delete = true,
        Action::Quit => app.quit(),
    }
}

/// Draws the open menu (if any) and runs the item clicked.
pub fn show_menu(app: &mut App, ctx: &Context) {
    if app.tray_menu.is_none() {
        return;
    }
    let list = entries(app);
    let Some(open) = app.tray_menu.as_mut() else { return };
    let size = vec2(WIDTH, height(&list));
    let screen = ctx.input(|i| i.viewport().monitor_size).unwrap_or(vec2(1920.0, 1080.0));
    let origin = menu_origin(open.at, size, screen);
    let mut clicked = None;
    let mut close = false;
    let mut hovered = false;
    let mut focused = None;
    ctx.show_viewport_immediate(
        ViewportId::from_hash_of("tray-menu"),
        ViewportBuilder::default()
            .with_title("Owlingstone menu")
            .with_position(origin)
            .with_inner_size(size)
            .with_decorations(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_active(true),
        |ui, class| {
            if class == ViewportClass::EmbeddedWindow {
                return;
            }
            let ctx = ui.ctx().clone();
            focused = ctx.input(|i| i.viewport().focused);
            hovered = ctx.input(|i| i.pointer.has_pointer());
            close |= ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested());
            let frame = egui::Frame::NONE
                .fill(draw::PANEL)
                .stroke(Stroke::new(1.0, draw::GOLD))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(PAD);
            egui::CentralPanel::default().frame(frame).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
                let p = ui.painter();
                p.text(pos2(head.left() + 8.0, head.center().y - 2.0), egui::Align2::LEFT_CENTER, "Owlingstone", draw::bold(14.0), draw::GOLD);
                p.text(
                    pos2(head.right() - 8.0, head.center().y - 2.0),
                    egui::Align2::RIGHT_CENTER,
                    format!("v{}", owlingstone::VERSION),
                    draw::font(11.0),
                    draw::GRAY,
                );
                p.hline(head.x_range(), head.bottom() - 3.0, Stroke::new(1.0, draw::GOLD.gamma_multiply(0.6)));
                for e in &list {
                    match e {
                        Entry::Separator => {
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), SEP_H), Sense::hover());
                            ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0, draw::BORDER));
                        }
                        Entry::Item { action, label, enabled, checked } => {
                            let sense = if *enabled { Sense::click() } else { Sense::hover() };
                            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ITEM_H), sense);
                            let hot = *enabled && resp.hovered();
                            let p = ui.painter();
                            if hot {
                                p.rect(r, CornerRadius::same(4), HOVER, Stroke::new(1.0, draw::GOLD.gamma_multiply(0.7)), egui::StrokeKind::Inside);
                            }
                            if *checked == Some(true) {
                                p.text(pos2(r.left() + 14.0, r.center().y), egui::Align2::CENTER_CENTER, "✓", draw::bold(13.0), draw::GOLD);
                            }
                            let color = if !*enabled {
                                draw::DARK_GRAY
                            } else if hot {
                                draw::LIGHT_GOLD
                            } else if *action == Action::Update {
                                draw::GOLD
                            } else {
                                draw::TEXT
                            };
                            p.text(pos2(r.left() + 28.0, r.center().y), egui::Align2::LEFT_CENTER, label, draw::font(13.0), color);
                            if resp.clicked() {
                                clicked = Some(*action);
                            }
                        }
                    }
                }
            });
            if hovered || clicked.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::Default);
            }
        },
    );
    let now = Instant::now();
    if hovered {
        open.last_hover = now;
    }
    match focused {
        Some(true) => open.focused = true,
        // Another window was clicked: a menu closes then.
        Some(false) if open.focused => close = true,
        _ => {}
    }
    if !open.focused && !hovered && now.duration_since(open.last_hover) > STALE && now.duration_since(open.opened) > STALE {
        close = true;
    }
    if clicked.is_some() || close {
        app.tray_menu = None;
    }
    if let Some(a) = clicked {
        apply(app, a);
    }
    ctx.request_repaint_after(Duration::from_millis(100));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_opens_above_the_mouse_and_stays_on_screen() {
        let size = vec2(270.0, 400.0);
        let screen = vec2(1920.0, 1080.0);
        // Tray at the bottom right: the menu goes up and left of the screen edge.
        let o = menu_origin(pos2(1900.0, 1060.0), size, screen);
        assert_eq!(o, pos2(1920.0 - 270.0 - 4.0, 660.0));
        // Taskbar at the top: never above the screen.
        assert_eq!(menu_origin(pos2(100.0, 20.0), size, screen), pos2(100.0, 0.0));
    }

    #[test]
    fn menu_height_counts_items_and_separators() {
        let list = vec![
            Entry::Item { action: Action::Quit, label: "Quit".into(), enabled: true, checked: None },
            Entry::Separator,
        ];
        assert_eq!(height(&list), HEADER_H + ITEM_H + SEP_H + 2.0 * PAD);
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::mpsc::{Receiver, channel};

    use super::super::App;
    use eframe::egui::{Context, pos2};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    pub struct Tray {
        _icon: TrayIcon,
        clicks: Receiver<TrayIconEvent>,
    }

    impl Tray {
        pub fn new(_app: &App, ctx: &Context) -> Option<Tray> {
            let data = super::super::icon();
            // No Windows menu: a click opens the app's own menu (show_menu).
            let mut b = TrayIconBuilder::new().with_tooltip(format!("Owlingstone v{}", owlingstone::VERSION));
            if let Ok(i) = Icon::from_rgba(data.rgba, data.width, data.height) {
                b = b.with_icon(i);
            }
            let icon = b.build().ok()?;
            let (tx, clicks) = channel();
            let ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                if matches!(e, TrayIconEvent::Click { .. }) {
                    let _ = tx.send(e);
                    ctx.request_repaint(); // the menu shows at once, not at the next 250 ms tick
                }
            }));
            Some(Tray { _icon: icon, clicks })
        }

        pub fn poll(&mut self, app: &mut App, ppp: f32) {
            while let Ok(e) = self.clicks.try_recv() {
                if let TrayIconEvent::Click { position, button, button_state: MouseButtonState::Up, .. } = e {
                    if matches!(button, MouseButton::Left | MouseButton::Right) {
                        // A second click on the icon closes the menu, like Windows menus.
                        app.tray_menu = match app.tray_menu {
                            Some(_) => None,
                            None => Some(super::OpenMenu::new(pos2(position.x as f32 / ppp, position.y as f32 / ppp))),
                        };
                    }
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::super::App;
    use eframe::egui::Context;
    pub struct Tray;
    impl Tray {
        pub fn new(_app: &App, _ctx: &Context) -> Option<Tray> {
            None
        }
        pub fn poll(&mut self, _app: &mut App, _ppp: f32) {}
    }
}

pub use imp::Tray;
