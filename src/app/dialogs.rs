//! Small windows: my stats, send feedback, what is sent, the sharing question, delete my shared games,
//! a message; and the short notices near the bottom right of the screen (toasts).

use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use eframe::egui::{self, Context, RichText, ViewportBuilder, ViewportClass, ViewportId};
use owlingstone::history;
use owlingstone::share::{self, Share};

use super::{App, draw};

pub const SHARE_QUESTION: &str = "Share anonymous game summaries to improve everyone's advice?\n\nWhen a game ends, Owlingstone would send your hero, placement, comp, lobby tribes, final board and the boards of each fight (cards and stats only). Never your BattleTag, other players' names, your logs or files. You can turn it off or delete what you sent at any time from the tray menu.";

#[derive(Default)]
pub struct Dialogs {
    /// Stats window open; the text is a note shown above the report (import result).
    pub stats: Option<String>,
    pub feedback: Option<(String, String, String)>,
    pub sent: bool,
    pub ask_sharing: bool,
    pub confirm_delete: bool,
    pub message: Option<(String, String)>,
    pub quit: bool,
    importing: Option<Receiver<String>>,
    deleting: Option<Receiver<bool>>,
}

fn dialog(title: &str, size: [f32; 2]) -> ViewportBuilder {
    ViewportBuilder::default().with_title(title).with_inner_size(size).with_always_on_top().with_resizable(true)
}

/// Shows the open dialogs (each in its own window).
pub fn show(app: &mut App, ctx: &Context) {
    stats(app, ctx);
    feedback(app, ctx);
    sent(app, ctx);
    ask_sharing(app, ctx);
    confirm_delete(app, ctx);
    message(app, ctx);
    toasts(app, ctx);
}

fn closed(ctx: &Context) -> bool {
    ctx.input(|i| i.viewport().close_requested())
}

fn stats(app: &mut App, ctx: &Context) {
    if let Some(rx) = &app.dialogs.importing {
        if let Ok(note) = rx.try_recv() {
            app.dialogs.importing = None;
            app.dialogs.stats = Some(note);
        }
    }
    let Some(note) = app.dialogs.stats.clone() else { return };
    let k = owlingstone::knowledge::get();
    let report = format!("{note}{}", history::format_report(&app.history(), &k.rules));
    let busy = app.dialogs.importing.is_some();
    let mut close = false;
    let mut import = false;
    ctx.show_viewport_immediate(ViewportId::from_hash_of("dlg-stats"), dialog("Owlingstone – my stats", [640.0, 560.0]), |ui, class| {
                let ctx = &ui.ctx().clone();
        if class == ViewportClass::EmbeddedWindow {
            return;
        }
        close |= closed(ctx);
        egui::Panel::bottom("b").show(ui, |ui| {
            ui.horizontal(|ui| {
                let label = if busy { "Importing…" } else { "Import old games from Hearthstone logs" };
                import |= ui.add_enabled(!busy, egui::Button::new(label)).clicked();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| close |= ui.button("Close").clicked());
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new(&report).monospace());
            });
        });
    });
    if import {
        let (tx, rx) = channel();
        app.dialogs.importing = Some(rx);
        let _ = std::thread::Builder::new().name("import".into()).spawn(move || {
            let note = match owlingstone::hslogs::find_logs_dir() {
                None => "Hearthstone's Logs folder not found (tray menu > Choose Hearthstone folder…).\n\n".to_string(),
                Some(dir) => {
                    let paths = log_files(&dir);
                    let k = owlingstone::knowledge::get();
                    match history::add_records(&history::import_logs(&paths, &k), &history::history_path()) {
                        Ok(n) => format!("{} log files read, {n} new games added.\n\n", paths.len()),
                        Err(e) => format!("Could not write the history: {e}\n\n"),
                    }
                }
            };
            let _ = tx.send(note);
        });
    }
    if close {
        app.dialogs.stats = None;
    }
}

/// `Hearthstone_*/Power*.log` in a Logs folder, sorted.
pub fn log_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for folder in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if !folder.file_name().to_string_lossy().starts_with("Hearthstone_") {
            continue;
        }
        for f in std::fs::read_dir(folder.path()).into_iter().flatten().flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if name.starts_with("Power") && name.ends_with(".log") {
                found.push(f.path());
            }
        }
    }
    found.sort();
    found
}

/// Tray > Send feedback: an idea, a bug or anything, sent with the next send.
fn feedback(app: &mut App, ctx: &Context) {
    let Some((mut kind, mut text, mut info)) = app.dialogs.feedback.clone() else { return };
    let (mut close, mut send) = (false, false);
    ctx.show_viewport_immediate(ViewportId::from_hash_of("dlg-feedback"), dialog("Owlingstone – send feedback", [420.0, 300.0]), |ui, class| {
                let ctx = &ui.ctx().clone();
        if class == ViewportClass::EmbeddedWindow {
            return;
        }
        close |= closed(ctx);
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label("Ideas, bugs, anything: it helps decide the next updates.");
            egui::ComboBox::from_id_salt("kind").selected_text(&kind).show_ui(ui, |ui| {
                for k in ["Idea", "Bug", "Other"] {
                    ui.selectable_value(&mut kind, k.to_string(), k);
                }
            });
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .hint_text("Write here (1000 characters max). Please don't include personal information.")
                    .char_limit(1000)
                    .desired_rows(8)
                    .desired_width(f32::INFINITY),
            );
            ui.label(RichText::new(&info).color(draw::GRAY));
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    send |= ui.button("Send").clicked();
                    close |= ui.button("Cancel").clicked();
                });
            });
        });
    });
    if send {
        match Share::default().add_feedback(&kind.to_lowercase(), &text) {
            Ok(()) => {
                owlingstone::snapshot::send_in_background(Share::default());
                close = true;
            }
            Err(_) => info = "Please write at least a few words.".into(),
        }
    }
    app.dialogs.feedback = if close { None } else { Some((kind, text, info)) };
}

pub fn new_feedback() -> (String, String, String) {
    ("Idea".into(), String::new(), "Sent anonymously with your next game, or when Owlingstone next starts.".into())
}

fn sent(app: &mut App, ctx: &Context) {
    if !app.dialogs.sent {
        return;
    }
    let text = std::fs::read_to_string(Share::default().last_path())
        .unwrap_or_else(|_| "Nothing sent yet. After your next game, the exact summary sent appears here.".into());
    let mut close = false;
    ctx.show_viewport_immediate(ViewportId::from_hash_of("dlg-sent"), dialog("Owlingstone – what is sent", [460.0, 520.0]), |ui, class| {
                let ctx = &ui.ctx().clone();
        if class == ViewportClass::EmbeddedWindow {
            return;
        }
        close |= closed(ctx);
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| ui.label(RichText::new(&text).monospace()));
        });
    });
    if close {
        app.dialogs.sent = false;
    }
}

/// Asked once. No by default (closing the window = no).
fn ask_sharing(app: &mut App, ctx: &Context) {
    if !app.dialogs.ask_sharing {
        return;
    }
    let mut answer: Option<bool> = None;
    ctx.show_viewport_immediate(ViewportId::from_hash_of("dlg-share"), dialog("Owlingstone", [440.0, 230.0]), |ui, class| {
                let ctx = &ui.ctx().clone();
        if class == ViewportClass::EmbeddedWindow {
            return;
        }
        if closed(ctx) {
            answer = Some(false);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label(SHARE_QUESTION);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("No").clicked() {
                        answer = Some(false);
                    }
                    if ui.button("Yes").clicked() {
                        answer = Some(true);
                    }
                });
            });
        });
    });
    if let Some(yes) = answer {
        app.dialogs.ask_sharing = false;
        if let Err(e) = Share::default().set_enabled(yes) {
            owlingstone::errlog::note("sharing.json", &e.to_string());
        }
    }
}

fn confirm_delete(app: &mut App, ctx: &Context) {
    if let Some(rx) = &app.dialogs.deleting {
        if let Ok(ok) = rx.try_recv() {
            app.dialogs.deleting = None;
            let text = if ok { "Your shared data was deleted." } else { "Could not reach the server. Try again later." };
            app.toast("Owlingstone", text);
        }
    }
    if !app.dialogs.confirm_delete {
        return;
    }
    let mut answer: Option<bool> = None;
    ctx.show_viewport_immediate(ViewportId::from_hash_of("dlg-delete"), dialog("Owlingstone", [380.0, 120.0]), |ui, class| {
                let ctx = &ui.ctx().clone();
        if class == ViewportClass::EmbeddedWindow {
            return;
        }
        if closed(ctx) {
            answer = Some(false);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label("Delete every game and feedback you shared from the server, and stop sharing?");
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("No").clicked() {
                    answer = Some(false);
                }
                if ui.button("Yes").clicked() {
                    answer = Some(true);
                }
            });
        });
    });
    if let Some(yes) = answer {
        app.dialogs.confirm_delete = false;
        if yes {
            let (tx, rx) = channel();
            app.dialogs.deleting = Some(rx);
            let _ = std::thread::Builder::new().name("share-delete".into()).spawn(move || {
                let _ = tx.send(Share::default().delete_remote(&share::http_send));
            });
        }
    }
}

fn message(app: &mut App, ctx: &Context) {
    let Some((title, text)) = app.dialogs.message.clone() else { return };
    let mut close = false;
    ctx.show_viewport_immediate(ViewportId::from_hash_of("dlg-message"), dialog(&title, [420.0, 200.0]), |ui, class| {
                let ctx = &ui.ctx().clone();
        if class == ViewportClass::EmbeddedWindow {
            return;
        }
        close |= closed(ctx);
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label(&text);
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| close |= ui.button("OK").clicked());
        });
    });
    if close {
        app.dialogs.message = None;
    }
}

/// The oldest notice still due, at the bottom right of the screen (click to close).
fn toasts(app: &mut App, ctx: &Context) {
    let now = Instant::now();
    app.toasts.retain(|t| t.until > now);
    let Some(t) = app.toasts.first() else { return };
    let (title, text) = (t.title.clone(), t.text.clone());
    let screen = ctx.input(|i| i.viewport().monitor_size).unwrap_or(egui::vec2(1920.0, 1080.0));
    let size = egui::vec2(360.0, 120.0);
    let pos = egui::pos2(screen.x - size.x - 16.0, screen.y - size.y - 60.0);
    let mut dismiss = false;
    ctx.show_viewport_immediate(
        ViewportId::from_hash_of("toast"),
        ViewportBuilder::default()
            .with_title("Owlingstone")
            .with_position(pos)
            .with_inner_size(size)
            .with_decorations(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_active(false),
        |ui, class| {
            if class == ViewportClass::EmbeddedWindow {
                return;
            }
            let frame = egui::Frame::NONE.fill(draw::PANEL).stroke(egui::Stroke::new(1.0, draw::GOLD)).inner_margin(10.0);
            egui::CentralPanel::default().frame(frame).show(ui, |ui| {
                ui.label(RichText::new(&title).font(draw::bold(13.0)).color(draw::GOLD));
                ui.label(&text);
                if ui.interact(ui.max_rect(), ui.scope_id().with("toast"), egui::Sense::click()).clicked() {
                    dismiss = true;
                }
            });
        },
    );
    if dismiss {
        app.toasts.remove(0);
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
}
