//! Right of the game, like HDT's Battlegrounds HUD, with two tabs.
//!
//! Comps tab: what I play (comp recognised, cards missing), the turn plan and buys, or the heroes to
//! pick; then every comp of this lobby as a tier list (S to D, my rules applied). A click opens a comp:
//! core cards / add-ons (green = mine, gold = in the shop), when to commit, how to play, my rules' notes.
//! The open comp's cards get ★ in the tier list on the left. Comps hidden by my rules are listed last.
//!
//! Lobby tab: the win chance of the fight, my next opponent (or the player clicked) with the board they
//! had the last time I fought them, then the 8 players by place. Under both tabs: my fights of this
//! game and my games of the day.

use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use owlingstone::cards::{self, Card};
use owlingstone::comps;
use owlingstone::knowledge::{self, Comp};
use owlingstone::planner::{CombatInfo, Odds};
use owlingstone::rules::{Ctx, Verdict};
use owlingstone::snapshot::{PlanAdvice, PlayerLine, RankedComp, Snapshot, TrinketAdvice};

use super::App;
use super::draw::{self, Hover, Rich, State, Tile};
use super::left::card_tile;

fn tier_list_color(letter: &str) -> Color32 {
    draw::hex(match letter {
        "S" => "ff7f7f",
        "A" => "ffbf7f",
        "B" => "ffdf7f",
        "C" => "bfff7f",
        "D" => "7fbfff",
        _ => "9aa0ad",
    })
}

pub fn show(app: &mut App, ui: &mut egui::Ui, hover: &mut Option<Hover>) {
    let ctx = &ui.ctx().clone();
    let panel = egui::Frame::NONE.fill(ctx.global_style().visuals.panel_fill).inner_margin(8.0).corner_radius(4);
    let snap = app.snap.clone();
    let folded_before = app.folded.clone();
    let tab_before = app.tab.clone();
    egui::CentralPanel::default().frame(panel).show(ui, |ui| {
        counters(ui, &snap);
        ui.columns(2, |cols| {
            for (i, name) in ["Comps", "Lobby"].into_iter().enumerate() {
                let on = app.tab == name;
                let mut text = RichText::new(name).strong();
                if on {
                    text = text.color(Color32::from_rgb(0x11, 0x11, 0x11));
                }
                let mut b = egui::Button::new(text).min_size(vec2(cols[i].available_width(), 24.0));
                if on {
                    b = b.fill(draw::GOLD);
                }
                if cols[i].add(b).clicked() {
                    app.tab = name.to_string();
                }
            }
        });
        egui::Panel::bottom("right-bottom").frame(egui::Frame::NONE).show(ui, |ui| {
            fights_line(ui, &snap);
            session_line(app, ui);
        });
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if app.tab == "Lobby" {
                lobby_tab(app, ui, &snap, hover);
            } else {
                comps_tab(app, ui, &snap, hover);
            }
        });
    });
    if app.folded != folded_before || app.tab != tab_before {
        app.save_settings();
    }
}

/// Counters like HDT's: turn, tier, bonus of my Tavern spells, value of my Blood Gems.
fn counters(ui: &mut egui::Ui, s: &Snapshot) {
    if !s.in_game {
        return;
    }
    let resp = ui
        .horizontal_wrapped(|ui| {
            draw::pill(ui, &format!("Turn {}", s.turn), draw::hex("b8962a"), Color32::from_rgb(0x11, 0x11, 0x11));
            if let Some(t) = s.tier {
                draw::pill(ui, &format!("Tier {t}"), draw::tier_color(Some(t)), Color32::WHITE);
            }
            let b = s.bonuses.unwrap_or(owlingstone::game::Bonuses { spell: (0, 0), gem: (1, 1) });
            let spell_on = b.spell != (0, 0);
            draw::pill(ui, &format!("✨ Tavern spells +{}/+{}", b.spell.0, b.spell.1), draw::hex(if spell_on { "2d4f7a" } else { "3a3e4a" }), Color32::WHITE);
            if s.races.as_ref().is_some_and(|r| r.iter().any(|x| x == "QUILBOAR")) || b.gem != (1, 1) {
                draw::pill(ui, &format!("🩸 Blood Gem +{}/+{}", b.gem.0, b.gem.1), draw::hex("7a2e3a"), Color32::WHITE);
            }
        })
        .response;
    resp.on_hover_text("Tavern spells: extra attack/health my Tavern spells give.\nBlood Gem: what one Blood Gem gives right now (+1/+1 at the start).");
}

// ------------------------------------------------------------------------------------- Comps tab
fn comps_tab(app: &mut App, ui: &mut egui::Ui, s: &Snapshot, hover: &mut Option<Hover>) {
    let k = knowledge::get();
    if !s.data_errors.is_empty() {
        for e in &s.data_errors {
            ui.label(RichText::new(e).color(draw::RED));
        }
    }
    if !s.in_game {
        let mut r = Rich::new().gray("Waiting for a Battlegrounds game…");
        if s.over {
            if let Some(p) = s.placement {
                r = r.br().gray("Last game: ").b(draw::ordinal(p));
            }
        }
        r.show(ui);
        return;
    }
    let owned: HashSet<&str> = s.board.iter().chain(&s.hand).map(String::as_str).collect();
    let shop: HashSet<&str> = s.shop.iter().map(String::as_str).collect();
    advice(app, ui, s, &owned, &shop, hover);
    if !s.advice.heroes.is_empty() {
        return;
    }
    // Tier list of the comps of this lobby.
    let title = if s.sure_races().is_some() { "COMPS IN THIS LOBBY" } else { "COMPS (RACES NOT KNOWN YET: ALL)" };
    if !draw::section(ui, &format!("{title}  ({})", s.advice.ranked.len()), "comps", &mut app.folded) {
        return;
    }
    let open = app.shown_comp();
    let mut current = "";
    for r in &s.advice.ranked {
        let Some(comp) = k.comp(&r.comp_id) else { continue };
        if r.letter != current {
            current = r.letter;
            let n = s.advice.ranked.iter().filter(|x| x.letter == r.letter).count();
            let key = format!("tier {}", r.letter.to_lowercase());
            let shown = !app.folded.contains(&key);
            let text = format!("{} {} tier · {n} comp{}", if shown { "▾" } else { "▸" }, r.letter, if n > 1 { "s" } else { "" });
            ui.add_space(3.0);
            if ui.add(egui::Label::new(RichText::new(text).font(draw::bold(12.0)).color(tier_list_color(r.letter))).sense(Sense::click())).clicked() {
                if shown {
                    app.folded.insert(key.clone());
                } else {
                    app.folded.remove(&key);
                }
            }
        }
        if app.folded.contains(&format!("tier {}", r.letter.to_lowercase())) {
            continue;
        }
        let is_open = open.as_deref() == Some(comp.id.as_str());
        if comp_card(app, ui, comp, r, &owned, &shop, is_open, hover) {
            // Click: open it, or close it if it was the open one.
            app.open_comp = Some(if is_open { None } else { Some(comp.id.clone()) });
        }
    }
    if !s.advice.hidden.is_empty() {
        let names: Vec<&str> = s.advice.hidden.iter().filter_map(|id| k.comp(id)).map(|c| c.name.as_str()).collect();
        ui.add_space(4.0);
        Rich::new().gray(format!("Hidden by my rules: {}", names.join(", "))).show(ui);
    }
}

/// Top of the Comps tab: heroes to pick, Trinkets, turn plan, comp played and missing cards, buys.
fn advice(app: &mut App, ui: &mut egui::Ui, s: &Snapshot, owned: &HashSet<&str>, shop: &HashSet<&str>, hover: &mut Option<Hover>) {
    let k = knowledge::get();
    let adv = &s.advice;
    let art_on = app.settings.card_art();
    if let Some(d) = &s.deity {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Deity:").color(draw::GRAY));
            draw::pill(ui, d, draw::hex("5b3a7a"), Color32::WHITE);
        });
    }
    if !s.hero_powers.is_empty() && adv.heroes.is_empty() {
        let title = if s.hero_powers.len() > 1 { "My hero powers" } else { "My hero power" };
        if draw::section(ui, title, "my hero power", &mut app.folded) {
            let db = cards::db();
            for (name, id) in &s.hero_powers {
                let card = db.get(id).cloned();
                Tile {
                    name: name.clone(),
                    art: if art_on { app.images.tile(id) } else { None },
                    gem: Some(card.as_ref().map(|c| c.cost.to_string()).unwrap_or_else(|| "?".into())),
                    gem_color: Some(draw::hex("2d4f7a")),
                    card_id: Some(id.clone()),
                    tooltip: card.map(|c| cards::plain_text(&c.text)).unwrap_or_default(),
                    ..Default::default()
                }
                .show(ui, hover);
            }
        }
    }
    if !adv.heroes.is_empty() {
        if draw::section(ui, "Pick a hero (avg place)", "pick a hero", &mut app.folded) {
            for (i, h) in adv.heroes.iter().enumerate() {
                let best = i == 0 && h.stat.is_some();
                let badge = match h.stat {
                    Some(st) => (format!("{:.2}", st.avg), draw::place_color(Some(st.avg))),
                    None => ("?".into(), draw::DARK_GRAY),
                };
                Tile {
                    name: h.name.clone(),
                    art: if art_on && !h.card_id.is_empty() { app.images.tile(&h.card_id) } else { None },
                    gem: Some((i + 1).to_string()),
                    gem_color: Some(if best { draw::hex("b8962a") } else { draw::BORDER }),
                    badges: vec![badge],
                    state: if best { State::Playing } else { State::None },
                    height: Some(34.0),
                    card_id: (!h.card_id.is_empty()).then(|| h.card_id.clone()),
                    ..Default::default()
                }
                .show(ui, hover);
                let mut r = Rich::new();
                if !h.best_comps.is_empty() {
                    r = r.gray("best in: ");
                    for (j, (id, avg, _)) in h.best_comps.iter().enumerate() {
                        if j > 0 {
                            r = r.t(" · ");
                        }
                        let name = k.comp(id).map(|c| c.name.clone()).unwrap_or_else(|| id.clone());
                        r = r.t(format!("{name} ")).bc(format!("{avg:.2}"), draw::place_color(Some(*avg)));
                    }
                }
                if !h.note.is_empty() {
                    if !r.is_empty() {
                        r = r.br();
                    }
                    r = r.gray(&h.note);
                }
                if !r.is_empty() {
                    r.show(ui);
                }
            }
        }
        return;
    }
    if let Some(t) = &adv.trinkets {
        trinkets(app, ui, t, hover);
    }
    if let Some(p) = &adv.plan {
        plan(app, ui, p);
    }
    if let Some(best) = &adv.playing {
        if draw::section(ui, &format!("Playing · {} · {}%", best.name, best.confidence), "playing", &mut app.folded) {
            for c in best.missing_core.iter().take(3) {
                let state = if shop.contains(c.id.as_str()) { State::Shop } else { State::None };
                card_tile(app, c, state, "Need").show(ui, hover);
            }
            if best.missing_core.is_empty() {
                ui.label(RichText::new("All core cards ✓").color(draw::GREEN));
            }
            if let Some(alt) = &adv.alt {
                ui.label(RichText::new(format!("or {} ({}%)", alt.name, alt.confidence)).color(draw::GRAY));
            }
        }
    }
    let _ = owned;
    if !adv.buy.is_empty() && draw::section(ui, "Buy", "buy", &mut app.folded) {
        let planned: HashSet<&str> = adv.plan.as_ref().map(|p| p.advice.plan_buys.iter().map(|c| c.id.as_str()).collect()).unwrap_or_default();
        for pick in &adv.buy {
            // With the turn plan: what my gold buys now in full colour, the alternative greyed.
            let spare = adv.plan.is_some() && !planned.contains(pick.card.id.as_str());
            let mut t = card_tile(app, &pick.card, if spare { State::None } else { State::Buy }, if spare { "" } else { "▶" });
            t.dim = spare;
            t.tooltip = format!("{}\n{}", pick.reasons.join(", "), cards::plain_text(&pick.card.text));
            let resp = t.show(ui, hover);
            if resp.hovered() && !pick.reasons.is_empty() {
                // The picture shows the card; the reasons stay readable under it.
                resp.on_hover_text(pick.reasons.join(", "));
            }
        }
    }
}

fn trinkets(app: &mut App, ui: &mut egui::Ui, t: &TrinketAdvice, hover: &mut Option<Hover>) {
    let art_on = app.settings.card_art();
    if !t.offer.is_empty() {
        if draw::section(ui, "Trinket · pick (expected place)", "trinket-offer", &mut app.folded) {
            for (i, p) in t.offer.iter().enumerate() {
                let best = i == 0 && p.affordable;
                Tile {
                    name: p.card.name.clone(),
                    art: if art_on { app.images.tile(&p.card.id) } else { None },
                    gem: Some(p.cost.to_string()),
                    gem_color: Some(draw::hex("b8962a")),
                    badges: vec![(format!("{:.2}", p.score), draw::place_color(Some(p.score)))],
                    state: if best { State::Playing } else { State::None },
                    mark: if best { "▶".into() } else { String::new() },
                    dim: !p.affordable,
                    card_id: Some(p.card.id.clone()),
                    tooltip: cards::plain_text(&p.card.text),
                    ..Default::default()
                }
                .show(ui, hover);
                Rich::new().gray(p.reasons.join(" · ")).show(ui);
            }
        }
        return;
    }
    if let Some(h) = &t.hint {
        if draw::section(ui, "Next Trinket offer", "trinket-hint", &mut app.folded) {
            let mut r = Rich::new();
            for (i, line) in h.lines.iter().enumerate() {
                if i > 0 {
                    r = r.br();
                }
                r = if i == 0 { r.gray(line) } else { r.t(line) };
            }
            r.show(ui).on_hover_text(
                "Trinket offers come at the start of turn 6 (Lesser) and turn 9 (Greater). Tribe Trinkets need 2 minions of the tribe on turn 6, 3 on turn 9 (board and hand when the turn starts). One Trinket of your main tribe is always offered; one tribe you are not in can show up 2 gold cheaper (pivot). Numbers = average place of players who took it (bgbuddy.gg).",
            );
        }
    }
}

fn strength_word(pct: f64) -> String {
    if pct >= 75.0 {
        format!("strong (top {:.0}%)", 100.0 - pct)
    } else if pct <= 25.0 {
        format!("weak (bottom {pct:.0}%)")
    } else {
        "average".into()
    }
}

/// Turn plan: danger for my health, next opponent expected, level or not, sell, roll, freeze.
fn plan(app: &mut App, ui: &mut egui::Ui, p: &PlanAdvice) {
    let f = &p.fight;
    let a = &p.advice;
    let s = &f.base;
    if !draw::section(ui, &format!("Turn plan · {} gold", f.gold), "turn plan", &mut app.folded) {
        return;
    }
    let k = knowledge::get();
    let resp = ui
        .vertical(|ui| {
            ui.horizontal_wrapped(|ui| {
                draw::pill(ui, &format!("Danger {}", a.danger.level), draw::hex(a.danger.color), Color32::from_rgb(0x11, 0x11, 0x11));
                ui.label(RichText::new(&a.danger.text).color(draw::GRAY));
            });
            let mut r = Rich::new().gray("Next fight vs ").t(f.opponent.clone().unwrap_or_else(|| "?".into()));
            r = r.gray(format!(" T{}", f.opp_tier));
            if let Some(h) = f.opp_health {
                r = r.gray(format!(", {h} hp"));
            }
            r = r.t(": ").b(format!("{:.0}%", s.win)).t(" win · ").b(format!("{:.0}%", s.tie)).t(" tie · ").b(format!("{:.0}%", s.loss)).t(" loss · expected ");
            r = r.bc(format!("-{:.1}", s.avg_taken), draw::LOSS);
            if let Some(cap) = f.cap {
                r = r.t(format!(" (cap {cap})"));
            }
            if s.lethal_me >= 0.5 {
                r = r.t(format!(" · ☠ {:.0}%", s.lethal_me));
            }
            r.show(ui);
            Rich::new().gray(format!("Their board: {} · my board {} for turn {} (with hand)", f.basis, strength_word(f.strength), f.turn)).show(ui);
            let mut r = Rich::new();
            match a.level {
                Some(true) => r = r.bc("⬆ LEVEL ", draw::GREEN).t(&a.level_why),
                Some(false) => r = r.bc("✋ DON'T LEVEL ", draw::ORANGE).gray(&a.level_why),
                None if f.upgrade_cost.is_some() => r = r.gray(format!("Level: {}", a.level_why)),
                None => {}
            }
            if let Some(sell) = &a.sell {
                r = r.br().bc("$ SELL ", draw::ORANGE).t(sell).gray(" to make room");
            }
            if let Some(roll) = &a.roll {
                r = r.br().bc("⟳ ROLL ", draw::BLUE).t(roll);
                if !a.roll_pro.is_empty() {
                    r = r.gray(format!(" · {}", a.roll_pro));
                }
            } else if !a.roll_pro.is_empty() {
                r = r.br().gray(format!("Roll: {}", a.roll_pro));
            }
            if let Some(freeze) = &a.freeze {
                r = r.br().bc("❄ FREEZE ", draw::ICE).t(freeze);
            }
            if let Some(power) = &a.power {
                r = r.br().bc("⚡ HERO POWER ", draw::GOLD).t(power);
            }
            let mut bits = Vec::new();
            if !a.hero_note.is_empty() {
                bits.push(a.hero_note.clone());
            }
            if !p.hero_best.is_empty() {
                let list: Vec<String> = p
                    .hero_best
                    .iter()
                    .map(|(id, avg, _)| format!("{} {avg:.2}", k.comp(id).map(|c| c.name.as_str()).unwrap_or(id)))
                    .collect();
                bits.push(format!("best comps with your hero here: {}", list.join(", ")));
            }
            if !bits.is_empty() {
                r = r.br().gray(format!("{}: {}", f.profile.hero.clone().unwrap_or_else(|| "Hero".into()), bits.join(" · ")));
            }
            if !r.is_empty() {
                r.show(ui);
            }
        })
        .response;
    resp.on_hover_text("Win chance and damage are simulated against the board your next opponent should have now: their last board seen, grown with the turn (learned from past games), or a typical board if you never fought them. Damage is capped like in the game. Your hand minions count as played.");
}

/// One comp like HDT's guides: name, badges, card thumbnails; open: all its cards, when to commit,
/// how to play. Returns true when clicked.
#[allow(clippy::too_many_arguments)]
fn comp_card(app: &mut App, ui: &mut egui::Ui, comp: &Comp, r: &RankedComp, owned: &HashSet<&str>, shop: &HashSet<&str>, is_open: bool, hover: &mut Option<Hover>) -> bool {
    let playing = r.playing;
    let stroke = if playing {
        Stroke::new(2.0, draw::hex("e0b830"))
    } else if is_open {
        Stroke::new(1.0, draw::GRAY)
    } else {
        Stroke::new(1.0, draw::hex("2c2f39"))
    };
    let cards: Vec<Arc<Card>> = comp.core.iter().chain(&comp.addons).cloned().collect();
    let have = cards.iter().filter(|c| owned.contains(c.id.as_str())).count();
    let frame = egui::Frame::NONE.fill(draw::CARD_BG).stroke(stroke).corner_radius(6).inner_margin(egui::Margin::symmetric(6, 5));
    let resp = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(&comp.name).font(draw::bold(13.0)).color(if playing { draw::LIGHT_GOLD } else { Color32::WHITE }));
            ui.horizontal_wrapped(|ui| {
                if playing {
                    draw::pill(ui, "PLAYING", draw::hex("b8962a"), Color32::from_rgb(0x11, 0x11, 0x11));
                }
                if r.verdict.bonus > 0.0 {
                    draw::pill(ui, "👍 my rule", draw::hex("2f6b3c"), Color32::WHITE);
                } else if r.verdict.bonus < 0.0 {
                    draw::pill(ui, "👎 my rule", draw::hex("7a2e2e"), Color32::WHITE);
                }
                if let Some(avg) = comp.stats.avg {
                    draw::pill(ui, &format!("{avg:.2}"), draw::place_color(Some(avg)), Color32::WHITE);
                }
                if !comp.difficulty.is_empty() {
                    draw::pill(ui, &comp.difficulty, draw::BORDER, Color32::WHITE);
                }
                if have > 0 {
                    draw::pill(ui, &format!("{have}/{} ✓", cards.len()), draw::hex("2f6b3c"), Color32::WHITE);
                }
            });
            if !is_open {
                thumbs(app, ui, &cards, owned, shop);
                return;
            }
            draw::label_section(ui, "Core cards");
            for c in &comp.core {
                card_tile(app, c, own_state(c, owned, shop), "").show(ui, hover);
            }
            if !comp.addons.is_empty() {
                draw::label_section(ui, "Add-ons");
                for c in &comp.addons {
                    card_tile(app, c, own_state(c, owned, shop), "").show(ui, hover);
                }
            }
            comp_texts(ui, comp, &r.verdict);
        })
        .response;
    let resp = ui.interact(resp.rect, ui.scope_id().with(("comp", &comp.id)), Sense::click());
    resp.on_hover_text("Click to open / close. The open comp's cards get ★ in the tier list.").clicked()
}

fn own_state(c: &Card, owned: &HashSet<&str>, shop: &HashSet<&str>) -> State {
    if owned.contains(c.id.as_str()) {
        State::Owned
    } else if shop.contains(c.id.as_str()) {
        State::Shop
    } else {
        State::None
    }
}

fn comp_texts(ui: &mut egui::Ui, comp: &Comp, verdict: &Verdict) {
    let mut r = Rich::new();
    let mut first = true;
    let mut sep = |r: Rich| {
        if first {
            first = false;
            r
        } else {
            r.br().br()
        }
    };
    if !comp.when_to_commit.is_empty() {
        r = sep(r).bc("Commit when", draw::LIGHT_GOLD).br().t(&comp.when_to_commit);
    }
    if !comp.how_to_play.is_empty() {
        r = sep(r).bc("How to play", draw::LIGHT_GOLD).br().t(&comp.how_to_play);
    }
    if let (Some(avg), Some(games)) = (comp.stats.avg, comp.stats.games) {
        if games > 0.0 {
            r = sep(r).gray(format!("Avg place {avg:.2} over {} games (community data)", thousands(games as i64)));
        }
    }
    for note in &verdict.notes {
        r = sep(r).bc("My rule: ", draw::GREEN).t(note);
    }
    if !r.is_empty() {
        r.show(ui);
    }
}

fn thousands(n: i64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A row of small card pictures (with their tier); green outline = mine, gold = in the shop.
fn thumbs(app: &mut App, ui: &mut egui::Ui, cards: &[Arc<Card>], owned: &HashSet<&str>, shop: &HashSet<&str>) {
    const H: f32 = 26.0;
    const MAX: usize = 6;
    let shown = &cards[..cards.len().min(MAX)];
    let more = cards.len().saturating_sub(MAX);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), H), Sense::hover());
    let n = shown.len() + usize::from(more > 0);
    if n == 0 {
        return;
    }
    let gap = 3.0;
    let tw = 58f32.min((rect.width() - gap * (n as f32 - 1.0)) / n as f32);
    let art_on = app.settings.card_art();
    let mut x = rect.left();
    for c in shown {
        let r = Rect::from_min_size(pos2(x, rect.top()), vec2(tw, H));
        let p = ui.painter();
        p.rect_filled(r, CornerRadius::same(3), draw::BG);
        match art_on.then(|| app.images.tile(&c.id)).flatten() {
            Some(tex) => {
                // The middle-right part of the banner: that's where the minion's face is.
                let [iw, ih] = tex.size();
                let scaled_w = iw as f32 * H / ih as f32;
                let sx = (scaled_w - tw - scaled_w * 0.12).max(0.0);
                let uv = Rect::from_min_max(pos2(sx / scaled_w, 0.0), pos2(((sx + tw) / scaled_w).min(1.0), 1.0));
                p.add(egui::epaint::RectShape::filled(r, CornerRadius::same(3), Color32::WHITE).with_texture(tex.id(), uv));
            }
            None => {
                // No art yet: the name on one line (cut to fit), above the tier gem.
                let mut job = egui::text::LayoutJob::single_section(
                    c.name.clone(),
                    egui::TextFormat::simple(draw::font(9.0), Color32::from_rgb(0xc0, 0xc4, 0xcc)),
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(tw - 4.0);
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                p.with_clip_rect(r).galley(pos2(r.left() + 2.0, r.top() + 1.0), galley, Color32::WHITE);
            }
        }
        let st = own_state(c, owned, shop);
        let stroke = match st.color() {
            Some(col) => Stroke::new(2.0, col),
            None => Stroke::new(1.0, draw::hex("2c2f39")),
        };
        p.rect_stroke(r, CornerRadius::same(3), stroke, StrokeKind::Inside);
        let g = pos2(x + 2.0 + 5.5, rect.bottom() - 2.0 - 5.5);
        p.circle_filled(g, 5.5, draw::tier_color(c.tier));
        p.text(g, Align2::CENTER_CENTER, c.tier.map(|t| t.to_string()).unwrap_or_else(|| "?".into()), draw::bold(8.0), Color32::WHITE);
        x += tw + gap;
    }
    if more > 0 {
        let r = Rect::from_min_size(pos2(x, rect.top()), vec2(tw.min(30.0), H));
        ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("+{more}"), draw::font(11.0), draw::GRAY);
    }
    resp.on_hover_text(cards.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "));
}

// ------------------------------------------------------------------------------------- Lobby tab
fn lobby_tab(app: &mut App, ui: &mut egui::Ui, s: &Snapshot, hover: &mut Option<Hover>) {
    if s.in_game {
        if let Some(c) = &s.combat {
            combat(app, ui, c);
        }
    }
    let open = draw::section(ui, "Opponent", "opponent", &mut app.folded);
    if open {
        opponent(app, ui, s, hover);
    }
    if draw::section(ui, "Lobby", "lobby", &mut app.folded) {
        if s.players.is_empty() {
            ui.label(RichText::new("Players appear when the game starts.").color(draw::GRAY));
        }
        for p in &s.players {
            let selected = app.selected_player.is_some() && p.id == app.selected_player;
            if player_tile(app, p, selected, None).show(ui, hover).clicked() {
                app.selected_player = if selected { None } else { p.id };
            }
        }
    }
}

fn player_tile(app: &mut App, p: &PlayerLine, selected: bool, height: Option<f32>) -> Tile {
    let mut badges = Vec::new();
    if !p.dead {
        badges.push((p.health.to_string(), draw::HEALTH));
    }
    if let Some(t) = p.tier.filter(|t| *t > 0) {
        badges.push((format!("T{t}"), draw::tier_color(Some(t))));
    }
    if p.triples > 0 {
        badges.push((format!("{}★", p.triples), draw::hex("8a6d1e")));
    }
    if let Some(f) = &p.fight {
        badges.push((f.result.to_string(), draw::result_color(f.result)));
    }
    let state = if selected {
        State::Selected
    } else if p.me {
        State::Me
    } else if p.next {
        State::Next
    } else {
        State::None
    };
    let mut tip = vec![p.hero.clone()];
    if let Some(f) = &p.fight {
        let dmg = if f.damage != 0 { format!(" ({} damage)", f.damage) } else { String::new() };
        tip.push(format!("Last fight, turn {}: {}{dmg}", f.turn, draw::result_word(f.result)));
    }
    Tile {
        name: p.hero.clone(),
        art: if app.settings.card_art() && !p.hero_id.is_empty() { app.images.tile(&p.hero_id) } else { None },
        gem: Some(p.place.filter(|x| *x > 0).map(|x| x.to_string()).unwrap_or_else(|| "?".into())),
        gem_color: Some(if p.next { draw::hex("b8962a") } else { draw::BORDER }),
        badges,
        state,
        mark: if p.next { "▶".into() } else { String::new() },
        dim: p.dead,
        height,
        card_id: (!p.hero_id.is_empty()).then(|| p.hero_id.clone()),
        tooltip: tip.join("\n"),
        ..Default::default()
    }
}

/// The player whose board is shown: the one clicked, else my next opponent, else (during the fight,
/// once the game picked the next one) the last one fought.
fn shown_player<'s>(app: &App, s: &'s Snapshot) -> (Option<&'s PlayerLine>, bool) {
    if let Some(p) = s.players.iter().find(|p| app.selected_player.is_some() && p.id == app.selected_player) {
        return (Some(p), false);
    }
    if let Some(p) = s.players.iter().find(|p| p.next) {
        return (Some(p), true);
    }
    let last = s.fights.last().and_then(|f| f.opponent);
    (s.players.iter().find(|p| last.is_some() && p.id == last), false)
}

fn opponent(app: &mut App, ui: &mut egui::Ui, s: &Snapshot, hover: &mut Option<Hover>) {
    if !s.in_game {
        Rich::new().b("Lobby").gray(" · waiting for a game…").show(ui);
        return;
    }
    let (player, is_next) = shown_player(app, s);
    let Some(player) = player.cloned() else {
        Rich::new().b("Next opponent").gray(" · not known yet").show(ui);
        return;
    };
    let label = if is_next { "NEXT OPPONENT" } else if player.me { "YOU" } else { "PLAYER" };
    ui.label(RichText::new(label).font(draw::bold(12.0)).color(draw::GOLD));
    if player_tile(app, &player, false, Some(40.0)).show(ui, hover).clicked() {
        app.selected_player = if app.selected_player == player.id { None } else { player.id };
    }
    if player.me {
        ui.label(RichText::new("That's you. Click another player to see their board.").color(draw::GRAY));
        return;
    }
    let Some(fight) = &player.fight else {
        ui.label(RichText::new("Not fought yet: board unknown.").color(draw::GRAY));
        return;
    };
    let db = cards::db();
    let board: Vec<Arc<Card>> = fight.board.iter().filter_map(|m| db.base(&m.card_id).cloned()).collect();
    let mut r = Rich::new().gray(format!("Their board on turn {}: ", fight.turn)).bc(draw::result_word(fight.result), draw::result_color(fight.result));
    if fight.damage != 0 {
        r = r.t(format!(" ({} dmg)", fight.damage));
    }
    // The comp of my data this board looks most like (without my rules: it's theirs).
    let k = knowledge::get();
    let ctx = Ctx { races: s.sure_races().map(<[String]>::to_vec), ..Default::default() };
    if let Some(m) = comps::match_comps(&k.comps, &board, &[], &[], &ctx, 1, owlingstone::advice::PLAYING_MIN_CONFIDENCE).first() {
        r = r.br().gray("Plays: ").b(&m.comp.name).gray(format!(" ({}%)", m.confidence));
    }
    r.show(ui);
    for m in &fight.board {
        let Some(card) = db.base(&m.card_id).cloned() else { continue };
        let mut t = card_tile(app, &card, State::None, "");
        if !card.is_pool_spell {
            t.badges = vec![(m.attack.to_string(), draw::ATTACK), (m.health.to_string(), draw::HEALTH)];
        }
        t.golden = m.golden;
        t.show(ui, hover);
    }
    if fight.board.is_empty() {
        ui.label(RichText::new("(empty board)").color(draw::GRAY));
    }
}

/// Win / tie / loss of the fight going on (or estimated against my next opponent's last board), damage,
/// risk of dying, and my average over the game.
fn combat(app: &mut App, ui: &mut egui::Ui, c: &CombatInfo) {
    if c.now.is_none() && c.fights.is_empty() {
        return;
    }
    if !draw::section(ui, "Win chance", "win chance", &mut app.folded) {
        return;
    }
    if let Some(now) = &c.now {
        let who = now.opponent.clone().unwrap_or_else(|| "opponent".into());
        if now.live {
            Rich::new().bc("COMBAT ", draw::GOLD).gray(format!("vs {who}")).show(ui);
        } else {
            let basis = now.basis.clone().unwrap_or_else(|| format!("their turn {} board", now.seen_turn.unwrap_or(0)));
            Rich::new().bc("NEXT FIGHT ", draw::GOLD).gray(format!("vs {who} · estimate vs {basis}")).show(ui);
        }
        let sim = &now.sim;
        draw::odds_bar(ui, sim.win, sim.tie, sim.loss, 22.0)
            .on_hover_text(format!("Win {:.1}% · Tie {:.1}% · Loss {:.1}% ({} simulated fights)", sim.win, sim.tie, sim.loss, sim.runs));
        let mut r = Rich::new();
        let mut parts = Vec::new();
        if sim.win > 0.0 {
            parts.push(format!("you deal {:.1}", sim.dealt));
        }
        if sim.loss > 0.0 {
            parts.push(format!("you take {:.1}", sim.taken));
        }
        if !parts.is_empty() {
            r = r.gray("Avg damage: ").t(parts.join(" · "));
        }
        if sim.lethal_me >= 0.5 {
            if !r.is_empty() {
                r = r.br();
            }
            r = r.bc(format!("☠ {:.0}% chance this fight kills you", sim.lethal_me), draw::LOSS);
        }
        if sim.lethal >= 0.5 {
            if !r.is_empty() {
                r = r.br();
            }
            r = r.bc(format!("{:.0}% chance to kill them", sim.lethal), draw::WIN);
        }
        if !sim.unknown.is_empty() {
            if !r.is_empty() {
                r = r.br();
            }
            r = r.gray(format!("Not simulated: {}", sim.unknown.iter().take(4).cloned().collect::<Vec<_>>().join(", ")));
        }
        if !r.is_empty() {
            r.show(ui);
        }
    }
    if let Some((win, _, _)) = c.avg {
        let n = c.fights.len();
        let tip: Vec<String> = c
            .fights
            .iter()
            .map(|f| format!("Turn {}: {:.0}% win / {:.0}% tie / {:.0}% loss → {}", f.turn, f.win, f.tie, f.loss, f.result))
            .collect();
        Rich::new()
            .gray(format!("This game ({n} fight{}): ", if n > 1 { "s" } else { "" }))
            .t("avg win chance ")
            .b(format!("{win:.0}%"))
            .t(format!(" · won {} (expected {:.1})", c.actual_wins, c.expected_wins))
            .show(ui)
            .on_hover_text(tip.join("\n"));
    }
}

fn fights_line(ui: &mut egui::Ui, s: &Snapshot) {
    if !s.in_game || s.fights.is_empty() {
        return;
    }
    let won = s.fights.iter().filter(|f| f.result == 'W').count();
    let lost = s.fights.iter().filter(|f| f.result == 'L').count();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("My fights ({won}-{lost}):")).color(draw::GRAY));
        let start = s.fights.len().saturating_sub(12);
        for f in &s.fights[start..] {
            if matches!(f.result, 'W' | 'L' | 'T') {
                draw::pill(ui, &f.result.to_string(), draw::result_color(f.result), Color32::WHITE);
            }
        }
    });
}

fn session_line(app: &mut App, ui: &mut egui::Ui) {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let history = app.history();
    let mut games: Vec<_> = history.iter().filter(|r| r.date == today).collect();
    games.sort_by(|a, b| a.time.cmp(&b.time));
    if games.is_empty() {
        ui.label(RichText::new("Today: no finished game yet.").color(draw::GRAY));
        return;
    }
    let places: Vec<i64> = games.iter().map(|g| g.placement).collect();
    let avg = places.iter().sum::<i64>() as f64 / places.len() as f64;
    let top4 = places.iter().filter(|p| **p <= 4).count();
    let listed: Vec<String> = places[places.len().saturating_sub(8)..].iter().map(|p| draw::ordinal(*p)).collect();
    let n = games.len();
    ui.label(
        RichText::new(format!("Today: {n} game{} · avg {avg:.1} · top 4: {top4}/{n}\n{}", if n > 1 { "s" } else { "" }, listed.join(", ")))
            .color(draw::GRAY),
    );
}

/// Small banner at the top of the game, above the opponent's portrait (like BobsBuddy in HDT).
pub fn odds_banner(ui: &mut egui::Ui, odds: Option<&Odds>) {
    let frame = egui::Frame::NONE.fill(draw::hex("15171c")).inner_margin(egui::Margin { left: 6, right: 6, top: 3, bottom: 4 });
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        let Some(now) = odds else { return };
        let who = now.opponent.clone().unwrap_or_else(|| "opponent".into());
        let mut r = Rich::new().bc(if now.live { "COMBAT " } else { "NEXT FIGHT " }, draw::GOLD).gray(format!("vs {who}"));
        if now.sim.lethal_me >= 0.5 {
            r = r.bc(format!(" ☠ {:.0}%", now.sim.lethal_me), draw::LOSS);
        } else if now.sim.lethal >= 0.5 {
            r = r.bc(format!(" lethal {:.0}%", now.sim.lethal), draw::WIN);
        }
        ui.vertical_centered(|ui| {
            r.show(ui);
        });
        draw::odds_bar(ui, now.sim.win, now.sim.tie, now.sim.loss, 18.0);
    });
}
