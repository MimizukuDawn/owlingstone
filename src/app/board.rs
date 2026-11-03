//! Board on hover: the mouse over a hero portrait of the game's leaderboard (left of the board) shows
//! that player's last board seen in a fight against me, with the card pictures and their stats then.
//!
//! - Where the portraits are: the game draws its play area 1.4 times as wide as it is high, centred in the
//!   window; the 8 portraits sit in a column near its left edge, from 15% to 89% of the height (one per
//!   leaderboard place, 1st at the top). `leaderboard_slot` turns a cursor position into a place.
//! - A small thread reads the cursor 20 times a second and wakes the window only when the place under it
//!   changes, so the overlay costs nothing while the mouse is elsewhere.
//! - The board window lets clicks through, and sits right of the game's own tooltip for the portrait.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Stroke, StrokeKind, pos2, vec2};
use owlingstone::cards;
use owlingstone::snapshot::{PlayerLine, Snapshot};

use super::draw::{self, Rich};
use super::win;

/// Play area width / height.
const AREA_RATIO: f32 = 1.4;
const TOP: f32 = 0.15;
const COLUMN_H: f32 = 0.74;
const SLOTS: usize = 8;
const COLUMN_LEFT: f32 = 0.03;
/// The portrait is about as wide as its slot is high; a little margin on the right.
const SLOT_W: f32 = 1.15;

/// Card pictures (256 x 388): where the printed attack and health gems are, and their radius (in widths).
pub const CARD_ASPECT: f32 = 388.0 / 256.0;
const ATTACK_AT: (f32, f32) = (0.145, 0.907);
const HEALTH_AT: (f32, f32) = (0.92, 0.907);
const GEM_R: f32 = 0.095;

const HEADER_H: f32 = 40.0;
const PAD: f32 = 8.0;

/// The leaderboard place (1-8) of the portrait at `cursor`, both in the same unit as `game` (x, y, w, h).
pub fn leaderboard_slot(game: (f32, f32, f32, f32), cursor: (f32, f32)) -> Option<i64> {
    let (x, y, w, h) = game;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let slot_h = h * COLUMN_H / SLOTS as f32;
    let left = x + (w - h * AREA_RATIO) / 2.0 + h * COLUMN_LEFT;
    let top = y + h * TOP;
    let (cx, cy) = cursor;
    if cx < left || cx > left + slot_h * SLOT_W || cy < top || cy >= top + slot_h * SLOTS as f32 {
        return None;
    }
    Some(((cy - top) / slot_h) as i64 + 1)
}

/// Where the board window goes for the portrait of `place` with `n` cards (points): (x, y, w, h), and the
/// width of one card. Right of the game's own tooltip, level with the portrait, inside the game.
pub fn board_geometry(game: (f32, f32, f32, f32), place: i64, n: usize) -> ((f32, f32, f32, f32), f32) {
    let (x, y, w, h) = game;
    let area_left = x + (w - h * AREA_RATIO) / 2.0;
    let x0 = (area_left + h * 0.50).max(x);
    let x1 = (area_left + h * 1.37).min(x + w);
    let n = n.max(1) as f32;
    let card_w = (h * 0.115).min(((x1 - x0) - 2.0 * PAD) / n).max(40.0);
    let width = (card_w * n + 2.0 * PAD).max(260.0);
    let height = HEADER_H + card_w * CARD_ASPECT + 2.0 * PAD;
    let slot_h = h * COLUMN_H / SLOTS as f32;
    let centre = y + h * TOP + slot_h * (place as f32 - 0.5);
    let top = (centre - height / 2.0).clamp(y + h * 0.02, (y + h - height).max(y));
    ((x0.round(), top.round(), width.round(), height.round()), card_w)
}

/// Follows the cursor over the leaderboard in the background.
pub struct Watch {
    /// Hearthstone's game area (physical pixels), or None.
    pub game: Arc<Mutex<Option<win::Rect>>>,
    /// Watch the cursor at all (overlay shown, in a game, setting on).
    pub active: Arc<AtomicBool>,
    /// The place under the cursor, 0 = none.
    pub slot: Arc<AtomicI64>,
}

impl Watch {
    pub fn start(ctx: &egui::Context) -> Watch {
        let w = Watch { game: Arc::default(), active: Arc::default(), slot: Arc::default() };
        let (game, active, slot, ctx) = (w.game.clone(), w.active.clone(), w.slot.clone(), ctx.clone());
        let _ = std::thread::Builder::new().name("leaderboard hover".into()).spawn(move || {
            loop {
                let rect = *game.lock().unwrap_or_else(|e| e.into_inner());
                let found = match (active.load(Ordering::Relaxed), rect, win::cursor_pos()) {
                    (true, Some((x, y, w, h)), Some((cx, cy))) => {
                        leaderboard_slot((x as f32, y as f32, w as f32, h as f32), (cx as f32, cy as f32)).unwrap_or(0)
                    }
                    _ => 0,
                };
                if slot.swap(found, Ordering::Relaxed) != found {
                    ctx.request_repaint();
                }
                std::thread::sleep(Duration::from_millis(if active.load(Ordering::Relaxed) { 50 } else { 300 }));
            }
        });
        w
    }

    pub fn place(&self) -> Option<i64> {
        Some(self.slot.load(Ordering::Relaxed)).filter(|p| *p > 0)
    }
}

/// The player at a leaderboard place, unless it's me (the game shows my own board).
pub fn player_at(s: &Snapshot, place: i64) -> Option<&PlayerLine> {
    s.players.iter().find(|p| p.place == Some(place) && !p.me)
}

/// Number of cards the window is sized for (one at least, for the "not fought yet" line).
pub fn card_count(p: &PlayerLine) -> usize {
    p.fight.as_ref().map(|f| f.board.len()).unwrap_or(0).max(1)
}

fn gem(painter: &egui::Painter, c: egui::Pos2, r: f32, text: &str, fill: Color32, size: f32) {
    painter.circle(c, r, fill, Stroke::new(1.5, Color32::from_black_alpha(200)));
    let shadow = Color32::from_black_alpha(230);
    for (dx, dy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
        painter.text(c + vec2(dx, dy), Align2::CENTER_CENTER, text, draw::bold(size), shadow);
    }
    painter.text(c, Align2::CENTER_CENTER, text, draw::bold(size), Color32::WHITE);
}

/// Draws the window's content: who, when, the result, then the cards.
pub fn show(ui: &mut egui::Ui, p: &PlayerLine, card_w: f32, images: &mut super::images::Images, art: bool) {
    let frame = egui::Frame::NONE
        .fill(Color32::from_rgba_unmultiplied(0x15, 0x17, 0x1c, 235))
        .stroke(Stroke::new(1.0, draw::BORDER))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(PAD as i8));
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        let mut head = Rich::new().bc(&p.hero, draw::GOLD);
        if let Some(t) = p.tier.filter(|t| *t > 0) {
            head = head.gray(format!("  ·  tier {t}"));
        }
        if !p.dead {
            head = head.gray(format!("  ·  {} health", p.health));
        }
        let Some(fight) = &p.fight else {
            head.br().gray("Not fought yet: board unknown.").show(ui);
            return;
        };
        let mut r = head.br().gray(format!("Last board seen, turn {}: ", fight.turn)).bc(draw::result_word(fight.result), draw::result_color(fight.result));
        if fight.damage != 0 {
            r = r.gray(format!(" ({} damage)", fight.damage));
        }
        r.show(ui);
        if fight.board.is_empty() {
            ui.label(egui::RichText::new("(empty board)").color(draw::GRAY));
            return;
        }
        let db = cards::db();
        let top = ui.max_rect().top() + HEADER_H;
        let left = ui.max_rect().left();
        let painter = ui.painter().clone();
        let card_h = card_w * CARD_ASPECT;
        for (i, m) in fight.board.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(left + i as f32 * card_w, top), vec2(card_w, card_h));
            let card = db.base(&m.card_id);
            let tex = if art { images.card(&m.card_id) } else { None };
            match &tex {
                Some(t) => {
                    painter.image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    let inner = rect.shrink(3.0);
                    painter.rect(inner, CornerRadius::same(6), draw::CARD_BG, Stroke::new(1.0, draw::BORDER), StrokeKind::Inside);
                    let name = card.map(|c| c.name.clone()).unwrap_or_else(|| m.card_id.clone());
                    let galley = painter.layout(name, draw::bold(11.0), draw::TEXT, inner.width() - 8.0);
                    painter.galley(pos2(inner.left() + 4.0, inner.center().y - galley.size().y / 2.0), galley, draw::TEXT);
                }
            }
            if m.golden {
                painter.rect_stroke(rect.shrink(2.0), CornerRadius::same(8), Stroke::new(2.5, draw::LIGHT_GOLD), StrokeKind::Inside);
            }
            if card.is_some_and(|c| c.is_pool_spell) {
                continue;
            }
            let r = card_w * GEM_R;
            let at = |(fx, fy): (f32, f32)| pos2(rect.left() + fx * card_w, rect.top() + fy * card_h);
            let size = (r * 1.15).max(10.0);
            gem(&painter, at(ATTACK_AT), r, &m.attack.to_string(), draw::ATTACK, size);
            gem(&painter, at(HEALTH_AT), r, &m.health.to_string(), draw::HEALTH, size);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1920 x 1080: play area 1512 wide from x = 204; portraits from y = 162, 94.3 high, x 236..344.
    const FHD: (f32, f32, f32, f32) = (0.0, 0.0, 1920.0, 1080.0);

    #[test]
    fn slots_of_a_full_hd_game() {
        assert_eq!(leaderboard_slot(FHD, (280.0, 170.0)), Some(1));
        assert_eq!(leaderboard_slot(FHD, (280.0, 162.0 + 94.3 * 3.5)), Some(4));
        assert_eq!(leaderboard_slot(FHD, (280.0, 950.0)), Some(8));
        assert_eq!(leaderboard_slot(FHD, (280.0, 100.0)), None, "above the column");
        assert_eq!(leaderboard_slot(FHD, (280.0, 1000.0)), None, "below the column");
        assert_eq!(leaderboard_slot(FHD, (200.0, 500.0)), None, "left of it");
        assert_eq!(leaderboard_slot(FHD, (600.0, 500.0)), None, "on the board");
    }

    #[test]
    fn slots_follow_the_window() {
        // Same game in a 1280 x 720 window at (100, 50): everything scales and moves with it.
        let g = (100.0, 50.0, 1280.0, 720.0);
        let s = 720.0 / 1080.0;
        assert_eq!(leaderboard_slot(g, (100.0 + 280.0 * s, 50.0 + 170.0 * s)), Some(1));
        assert_eq!(leaderboard_slot(g, (100.0 + 280.0 * s, 50.0 + 950.0 * s)), Some(8));
        assert_eq!(leaderboard_slot((0.0, 0.0, 0.0, 0.0), (1.0, 1.0)), None);
    }

    #[test]
    fn board_stays_in_the_game_right_of_the_portraits() {
        for place in 1..=8 {
            for n in [1, 4, 7] {
                let ((x, y, w, h), card_w) = board_geometry(FHD, place, n);
                assert!(x > 344.0 + 150.0, "leaves room for the game's own tooltip");
                assert!(x + w <= 1920.0 && y >= 0.0 && y + h <= 1080.0, "place {place}, {n} cards: {x},{y},{w},{h}");
                assert!(card_w * n as f32 <= w);
            }
        }
        // Seven cards still readable on a full HD screen.
        assert!(board_geometry(FHD, 1, 7).1 >= 110.0);
    }
}
