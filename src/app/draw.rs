//! Drawing pieces in the style of Hearthstone Deck Tracker, shared by the tier list and the right panel.
//!
//! - `Tile`: one "card" row painted by hand: a gem on the left (tier, place...), the name in white with a
//!   shadow, the card's banner on the right fading into the background, round badges (attack / health,
//!   health, tier...). The outline says the state: gold = in the shop, green = mine / to buy, blue = in hand.
//! - `Rich`: a line of coloured text pieces (what the Python version wrote as HTML).
//! - `odds_bar`: win / tie / loss bar like BobsBuddy's, with the three numbers under it.
//! - `pill`: small coloured label (difficulty, average place...).

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Mesh, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, TextFormat, TextureHandle, Ui, pos2, text::LayoutJob, vec2,
};

pub const BG: Color32 = Color32::from_rgb(0x1c, 0x1e, 0x25);
pub const PANEL: Color32 = Color32::from_rgb(0x1b, 0x1d, 0x23);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe6, 0xe6);
pub const GRAY: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xad);
pub const DARK_GRAY: Color32 = Color32::from_rgb(0x5c, 0x60, 0x6b);
pub const GOLD: Color32 = Color32::from_rgb(0xc9, 0xa2, 0x27);
pub const LIGHT_GOLD: Color32 = Color32::from_rgb(0xf0, 0xd2, 0x7a);
pub const GREEN: Color32 = Color32::from_rgb(0x7f, 0xe0, 0x7f);
pub const SOFT_GREEN: Color32 = Color32::from_rgb(0x7f, 0xc9, 0x7f);
pub const RED: Color32 = Color32::from_rgb(0xe5, 0x73, 0x73);
pub const ORANGE: Color32 = Color32::from_rgb(0xe0, 0xa0, 0x50);
pub const BLUE: Color32 = Color32::from_rgb(0x7f, 0xb0, 0xe0);
pub const ICE: Color32 = Color32::from_rgb(0x9f, 0xd8, 0xff);
pub const BUTTON: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x36);
pub const BORDER: Color32 = Color32::from_rgb(0x3a, 0x3e, 0x4a);
pub const CARD_BG: Color32 = Color32::from_rgb(0x20, 0x23, 0x2b);
pub const ATTACK: Color32 = Color32::from_rgb(0xd9, 0xa5, 0x21);
pub const HEALTH: Color32 = Color32::from_rgb(0xc4, 0x3c, 0x3c);
pub const WIN: Color32 = Color32::from_rgb(0x3f, 0x9d, 0x55);
pub const TIE: Color32 = Color32::from_rgb(0x5c, 0x60, 0x6b);
pub const LOSS: Color32 = Color32::from_rgb(0xc4, 0x3c, 0x3c);

pub fn hex(s: &str) -> Color32 {
    let s = s.trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).unwrap_or(0x5c606b);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub fn tier_color(tier: Option<i64>) -> Color32 {
    match tier {
        Some(1) => hex("8a8f99"),
        Some(2) => hex("3f9d55"),
        Some(3) => hex("3a7bd5"),
        Some(4) => hex("9b59d0"),
        Some(5) => hex("e08a1e"),
        Some(6) => hex("d64545"),
        Some(7) => hex("e05bb5"),
        _ => DARK_GRAY,
    }
}

/// Colour of an average place: green = very good, yellow = average, red = weak.
pub fn place_color(avg: Option<f64>) -> Color32 {
    match avg {
        None => DARK_GRAY,
        Some(a) if a < 3.4 => hex("3f9d55"),
        Some(a) if a < 3.8 => hex("b8962a"),
        Some(_) => hex("b5473f"),
    }
}

pub fn result_color(r: char) -> Color32 {
    match r {
        'W' => WIN,
        'L' => LOSS,
        _ => TIE,
    }
}

pub fn result_word(r: char) -> &'static str {
    match r {
        'W' => "you won",
        'L' => "you lost",
        _ => "tie",
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum State {
    #[default]
    None,
    Shop,
    Board,
    Owned,
    Buy,
    Hand,
    Next,
    Me,
    Playing,
    Selected,
}

impl State {
    pub fn color(self) -> Option<Color32> {
        Some(match self {
            State::None => return None,
            State::Shop | State::Next | State::Playing => hex("e0b830"),
            State::Board | State::Owned | State::Me => hex("4cbb5c"),
            State::Buy => GREEN,
            State::Hand => hex("4a90d9"),
            State::Selected => Color32::WHITE,
        })
    }
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

fn shadow_text(painter: &egui::Painter, pos: Pos2, align: Align2, text: &str, font: FontId, color: Color32) {
    let shadow = Color32::from_black_alpha(220);
    for (dx, dy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (0.0, 2.0)] {
        painter.text(pos + vec2(dx, dy), align, text, font.clone(), shadow);
    }
    painter.text(pos, align, text, font, color);
}

/// Cuts `text` with "…" to fit `width`.
pub fn elide(ui: &Ui, text: &str, font: &FontId, width: f32) -> String {
    let fits = |t: &str| ui.fonts_mut(|f| f.layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x) <= width;
    if fits(text) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let t: String = chars[..mid].iter().collect::<String>() + "…";
        if fits(&t) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

/// A row like HDT's. Badges are drawn right to left.
#[derive(Default)]
pub struct Tile {
    pub name: String,
    pub art: Option<TextureHandle>,
    pub gem: Option<String>,
    pub gem_color: Option<Color32>,
    pub badges: Vec<(String, Color32)>,
    pub state: State,
    pub mark: String,
    pub golden: bool,
    pub dim: bool,
    pub height: Option<f32>,
    /// Card (or hero) shown big when the mouse is over the row.
    pub card_id: Option<String>,
    pub tooltip: String,
}

pub const TILE_H: f32 = 30.0;

/// The card under the mouse this frame, and where (screen points), for the big picture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hover {
    pub card_id: String,
    pub tooltip: String,
    pub y: f32,
}

impl Tile {
    pub fn show(self, ui: &mut Ui, hover: &mut Option<Hover>) -> Response {
        let h = self.height.unwrap_or(TILE_H);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(120.0), h), Sense::click());
        if !ui.is_rect_visible(rect) {
            return resp;
        }
        let p = ui.painter_at(rect.expand(1.0));
        let r = CornerRadius::same(4);
        p.rect_filled(rect, r, BG);
        let w = rect.width();
        // Card banner on the right, faded to the left into the background (like HDT's banners).
        if let Some(art) = &self.art {
            let [tw, th] = art.size();
            let (tw, th) = (tw as f32 * 0.7, th as f32);
            let scale = h / th;
            let x0 = (w * 0.30).max(w - tw * scale * 1.25);
            let area = Rect::from_min_max(pos2(rect.left() + x0, rect.top()), rect.max);
            let a = tw / th;
            let t = area.width() / area.height();
            let uv = Rect::from_min_max(pos2(0.3, 0.0), pos2(0.3 + 0.7 * (t / a).min(1.0), (a / t).min(1.0)));
            p.add(egui::epaint::RectShape::filled(area, CornerRadius { ne: 4, se: 4, nw: 0, sw: 0 }, Color32::WHITE).with_texture(art.id(), uv));
            let len = area.width() * 0.75;
            let stops = [(0.0, 255u8), (0.25, 230), (0.5, 150), (0.8, 50), (1.0, 20)];
            let mut mesh = Mesh::default();
            let mut add = |x: f32, alpha: u8| {
                let c = Color32::from_rgba_unmultiplied(BG.r(), BG.g(), BG.b(), alpha);
                mesh.colored_vertex(pos2(x, area.top()), c);
                mesh.colored_vertex(pos2(x, area.bottom()), c);
            };
            add(area.left() - 1.0, 255);
            for (at, alpha) in stops {
                add(area.left() + at * len, alpha);
            }
            add(area.right(), 20);
            let n = mesh.vertices.len() as u32 / 2;
            for i in 0..n - 1 {
                let k = i * 2;
                mesh.add_triangle(k, k + 1, k + 2);
                mesh.add_triangle(k + 1, k + 3, k + 2);
            }
            p.add(Shape::mesh(mesh));
        }
        if self.golden {
            p.rect_filled(rect, r, Color32::from_rgba_unmultiplied(224, 184, 48, 40));
        }
        if self.dim {
            p.rect_filled(rect, r, Color32::from_black_alpha(120));
        }
        let hovered = resp.hovered();
        let stroke = match self.state.color() {
            Some(c) => Stroke::new(2.0, c),
            None if hovered => Stroke::new(1.0, GRAY),
            None => Stroke::new(1.0, hex("2c2f39")),
        };
        p.rect_stroke(rect, r, stroke, StrokeKind::Inside);
        let mut x = rect.left() + 4.0;
        if let Some(gem) = &self.gem {
            let d = h - 8.0;
            let c = pos2(x + d / 2.0, rect.center().y);
            p.circle(c, d / 2.0, self.gem_color.unwrap_or(DARK_GRAY), Stroke::new(1.5, Color32::from_black_alpha(160)));
            shadow_text(&p, c, Align2::CENTER_CENTER, gem, bold(11.0), Color32::WHITE);
            x += d + 6.0;
        }
        let mut right = rect.right() - 4.0;
        for (text, color) in self.badges.iter().rev() {
            let galley_w = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), bold(10.0), Color32::WHITE).size().x);
            let bw = (h - 10.0).max(galley_w + 10.0);
            let b = Rect::from_min_size(pos2(right - bw, rect.top() + 5.0), vec2(bw, h - 10.0));
            p.rect(b, CornerRadius::same(((h - 10.0) / 2.0) as u8), *color, Stroke::new(1.0, Color32::from_black_alpha(170)), StrokeKind::Inside);
            shadow_text(&p, b.center(), Align2::CENTER_CENTER, text, bold(10.0), Color32::WHITE);
            right -= bw + 3.0;
        }
        let label = if self.mark.is_empty() { self.name.clone() } else { format!("{} {}", self.mark, self.name) };
        let f = bold(12.0);
        let label = elide(ui, &label, &f, right - x - 4.0);
        let color = if self.golden {
            LIGHT_GOLD
        } else if self.dim {
            GRAY
        } else {
            Color32::WHITE
        };
        shadow_text(&p, pos2(x, rect.center().y), Align2::LEFT_CENTER, &label, f, color);
        if hovered {
            if let Some(id) = &self.card_id {
                *hover = Some(Hover { card_id: id.clone(), tooltip: self.tooltip.clone(), y: rect.center().y });
            } else if !self.tooltip.is_empty() {
                return resp.on_hover_text(self.tooltip);
            }
        }
        resp
    }
}

/// Coloured text pieces on one paragraph (wraps).
#[derive(Default)]
pub struct Rich(pub LayoutJob);

impl Rich {
    pub fn new() -> Rich {
        let mut job = LayoutJob::default();
        job.wrap.max_width = f32::INFINITY;
        Rich(job)
    }
    pub fn add(mut self, text: impl AsRef<str>, color: Color32, strong: bool, size: f32) -> Rich {
        let font = if strong { bold(size) } else { font(size) };
        self.0.append(text.as_ref(), 0.0, TextFormat { font_id: font, color, ..Default::default() });
        self
    }
    pub fn t(self, text: impl AsRef<str>) -> Rich {
        self.add(text, TEXT, false, 12.0)
    }
    pub fn gray(self, text: impl AsRef<str>) -> Rich {
        self.add(text, GRAY, false, 12.0)
    }
    pub fn b(self, text: impl AsRef<str>) -> Rich {
        self.add(text, TEXT, true, 12.0)
    }
    pub fn c(self, text: impl AsRef<str>, color: Color32) -> Rich {
        self.add(text, color, false, 12.0)
    }
    pub fn bc(self, text: impl AsRef<str>, color: Color32) -> Rich {
        self.add(text, color, true, 12.0)
    }
    pub fn br(self) -> Rich {
        self.add("\n", TEXT, false, 12.0)
    }
    pub fn is_empty(&self) -> bool {
        self.0.text.is_empty()
    }
    pub fn show(self, ui: &mut Ui) -> Response {
        let mut job = self.0;
        job.wrap.max_width = ui.available_width();
        ui.add(egui::Label::new(job).wrap())
    }
}

/// A small coloured label, inline.
pub fn pill(ui: &mut Ui, text: &str, bg: Color32, fg: Color32) -> Response {
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), bold(11.0), fg));
    let size = galley.size() + vec2(10.0, 4.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(3), bg);
    p.galley(rect.min + vec2(5.0, 2.0), galley, fg);
    resp
}

pub const ODDS_LABEL_H: f32 = 15.0;

/// Win / tie / loss bar with the three numbers always written under it.
pub fn odds_bar(ui: &mut Ui, win: f64, tie: f64, loss: f64, bar_h: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), bar_h + ODDS_LABEL_H), Sense::hover());
    let p = ui.painter_at(rect);
    let w = rect.width();
    let mut x = rect.left();
    for (i, (pct, color)) in [(win, WIN), (tie, TIE), (loss, LOSS)].into_iter().enumerate() {
        let width = if i < 2 { w * pct as f32 / 100.0 } else { rect.right() - x };
        if width <= 0.0 {
            continue;
        }
        p.rect_filled(Rect::from_min_size(pos2(x, rect.top()), vec2(width, bar_h)), 0.0, color);
        x += width;
    }
    let y = rect.top() + bar_h + ODDS_LABEL_H / 2.0;
    let f = bold(11.0);
    p.text(pos2(rect.left(), y), Align2::LEFT_CENTER, format!("Win {win:.0}%"), f.clone(), hex("7fe07f"));
    p.text(pos2(rect.center().x, y), Align2::CENTER_CENTER, format!("Tie {tie:.0}%"), f.clone(), hex("c8ccd4"));
    p.text(pos2(rect.right(), y), Align2::RIGHT_CENTER, format!("Loss {loss:.0}%"), f, hex("ff7a7a"));
    resp
}

/// Section title; clicking folds / unfolds what follows. Returns true if the section is open.
pub fn section(ui: &mut Ui, title: &str, key: &str, folded: &mut std::collections::BTreeSet<String>) -> bool {
    let open = !folded.contains(key);
    ui.add_space(4.0);
    let text = format!("{} {}", if open { "▾" } else { "▸" }, title.to_uppercase());
    let resp = ui
        .add(egui::Label::new(egui::RichText::new(text).font(bold(12.0)).color(GOLD)).sense(Sense::click()))
        .on_hover_text("Click to fold / unfold");
    if resp.clicked() {
        if open {
            folded.insert(key.to_string());
        } else {
            folded.remove(key);
        }
    }
    open
}

/// Simple title (not foldable).
pub fn label_section(ui: &mut Ui, title: &str) {
    ui.add_space(3.0);
    ui.label(egui::RichText::new(title.to_uppercase()).font(bold(11.0)).color(GOLD));
}

/// Ordinal of a place ("1st", "2nd"...).
pub fn ordinal(n: i64) -> String {
    let suffix = match n {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}
