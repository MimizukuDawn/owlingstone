//! Odds on the choice on screen: when the game asks me to pick (hero, Trinket, Discover), a strip over the
//! options shows each one's chance to be the best pick, the pick advised and why (`choices.rs`).
//!
//! - Where: just under the win chance banner, centred on the game, one column per option in the order
//!   the game shows them, spaced like the cards below (`COLUMN_H` heights apart, a guess from screenshots
//!   to fix if the columns sit off the cards). It covers only the top of the screen, where nothing is
//!   clicked during a choice.
//! - The × closes it for this choice; the tray menu turns it off for good ("Odds on choices").

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Stroke, vec2};
use owlingstone::choices::{ChoiceAdvice, Kind};

use super::draw;

/// Distance between two option columns, in game heights.
const COLUMN_H: f32 = 0.28;
const COLUMN_MAX: f32 = 320.0;
const COLUMN_MIN: f32 = 150.0;
/// Top of the strip, in game heights (the win chance banner is above).
const TOP: f32 = 0.072;
const HEADER_H: f32 = 22.0;
pub const STRIP_H: f32 = 132.0;

/// (x, y, w, h) of the strip for `n` options over the game area `game` (points).
pub fn geometry(game: (f32, f32, f32, f32), n: usize) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = game;
    let n = n.max(1) as f32;
    let col = (h * COLUMN_H).clamp(COLUMN_MIN, COLUMN_MAX).min((w - 20.0) / n);
    let width = col * n;
    ((x + (w - width) / 2.0).round(), (y + h * TOP).round(), width.round(), STRIP_H)
}

/// What identifies a choice (the × hides this one only).
pub fn key(c: &ChoiceAdvice) -> String {
    let ids: Vec<&str> = c.options.iter().map(|o| o.card_id.as_str()).collect();
    format!("{}|{}", c.title, ids.join(","))
}

fn chance_color(chance: f64, best: bool) -> Color32 {
    if best {
        draw::GREEN
    } else if chance >= 25.0 {
        draw::TEXT
    } else {
        draw::GRAY
    }
}

/// Draws the strip. Returns true when the × was clicked.
pub fn show(ui: &mut egui::Ui, c: &ChoiceAdvice) -> bool {
    let mut closed = false;
    let frame = egui::Frame::NONE.fill(draw::hex("15171c")).inner_margin(egui::Margin::same(4));
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
        ui.allocate_ui_with_layout(vec2(ui.available_width(), HEADER_H), Layout::left_to_right(Align::Center), |ui| {
            ui.label(RichText::new(c.title.to_uppercase()).color(draw::GOLD).font(draw::bold(12.0)));
            let note = match (c.sure, c.kind) {
                (false, _) => "no data on these options: even odds",
                (true, Kind::Discover) => "chance to be the best pick · educated guess",
                (true, _) => "chance to be the best pick",
            };
            ui.label(RichText::new(note).color(draw::GRAY).font(draw::font(11.0)));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let x = ui.add(egui::Button::new(RichText::new("×").color(draw::GRAY).font(draw::bold(14.0))).frame(false));
                if x.on_hover_text("Hide for this choice").clicked() {
                    closed = true;
                }
            });
        });
        let n = c.options.len().max(1) as f32;
        let col_w = (ui.available_width() - 4.0 * (n - 1.0)) / n;
        let col_h = ui.available_height();
        ui.horizontal(|ui| {
            for o in &c.options {
                let stroke = if o.best { Stroke::new(2.0, draw::GOLD) } else { Stroke::new(1.0, draw::BORDER) };
                egui::Frame::NONE
                    .fill(draw::PANEL)
                    .stroke(stroke)
                    .corner_radius(CornerRadius::same(5))
                    .inner_margin(egui::Margin::symmetric(6, 3))
                    .show(ui, |ui| {
                        ui.set_width(col_w - 12.0 - 2.0 * stroke.width);
                        ui.set_height(col_h - 6.0 - 2.0 * stroke.width);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            let name_font = draw::bold(13.0);
                            let name = draw::elide(ui, &o.name, &name_font, ui.available_width());
                            ui.label(RichText::new(name).color(draw::TEXT).font(name_font));
                            ui.horizontal(|ui| {
                                let pct = if c.sure { format!("{:.0}%", o.chance) } else { "?".into() };
                                ui.label(RichText::new(pct).color(chance_color(o.chance, o.best)).font(draw::bold(22.0)));
                                if o.best {
                                    ui.label(RichText::new("▶ PICK").color(draw::LIGHT_GOLD).font(draw::bold(13.0)));
                                }
                                ui.label(RichText::new(&o.score).color(draw::GRAY).font(draw::font(11.0)));
                            });
                            let why = o.reasons.iter().take(2).cloned().collect::<Vec<_>>().join(" · ");
                            if !why.is_empty() {
                                ui.add(egui::Label::new(RichText::new(why).color(draw::GRAY).font(draw::font(11.0))).wrap());
                            }
                        });
                    });
            }
        });
    });
    closed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_is_centred_and_fits() {
        let (x, y, w, h) = geometry((0.0, 0.0, 1920.0, 1080.0), 3);
        assert_eq!((x + w / 2.0).round(), 960.0);
        assert!(w <= 3.0 * COLUMN_MAX && y > 60.0 && h == STRIP_H);
        let (x, _, w, _) = geometry((100.0, 0.0, 800.0, 600.0), 4);
        assert!(x >= 100.0 && x + w <= 900.0);
    }
}
