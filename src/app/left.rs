//! Left of the game: minions of the lobby by tier, filtered by the game's tribes (like HDT's minion
//! browser). Header (hero, tier, turn, health, gold), tribes, tier buttons 1-7, keyword filter, then the
//! cards by section. Row colours: gold = in the shop, green = on my board, blue = in my hand; ▶ = buy,
//! ★ = card of the comp opened on the right.

use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};
use owlingstone::cards::{self, Card};
use owlingstone::pool;
use owlingstone::snapshot::Snapshot;

use super::App;
use super::draw::{self, Hover, Rich, State, Tile};

/// Comps of the lobby listed at the start of a game.
const LOBBY_SHOWN: usize = owlingstone::snapshot::LOBBY_SHOWN;

pub fn card_tile(app: &mut App, card: &Arc<Card>, state: State, mark: &str) -> Tile {
    let badges = if card.is_pool_spell {
        vec![(format!("{}g", card.cost), draw::hex("8a6d1e"))]
    } else {
        vec![(card.attack.to_string(), draw::ATTACK), (card.health.to_string(), draw::HEALTH)]
    };
    Tile {
        name: card.name.clone(),
        art: if app.settings.card_art() { app.images.tile(&card.id) } else { None },
        gem: Some(card.tier.map(|t| t.to_string()).unwrap_or_else(|| "?".into())),
        gem_color: Some(draw::tier_color(card.tier)),
        badges,
        state,
        mark: mark.to_string(),
        card_id: Some(card.id.clone()),
        tooltip: cards::plain_text(&card.text),
        ..Default::default()
    }
}

fn header(ui: &mut egui::Ui, s: &Snapshot) {
    if !s.in_game {
        let mut text = Rich::new().gray(s.waiting_text());
        if s.over {
            if let Some(p) = s.placement {
                text = text.br().gray("Last game: ").b(format!("#{p}"));
            }
        }
        text.show(ui);
        return;
    }
    let mut r = Rich::new().b(s.hero.clone().unwrap_or_else(|| "Choosing hero…".into()));
    r = r.gray(" · ").t(format!("Tier {}", s.tier.map(|t| t.to_string()).unwrap_or_else(|| "?".into())));
    r = r.gray(" · ").t(format!("Turn {}", s.turn));
    if let Some(h) = s.health {
        r = r.gray(" · ").t(format!("❤ {h}"));
    }
    if let Some(g) = s.gold {
        r = r.gray(" · ").t(format!("🪙 {g}"));
    }
    r.show(ui);
}

fn races_line(ui: &mut egui::Ui, s: &Snapshot) {
    let Some(races) = s.races.as_ref().filter(|r| !r.is_empty()) else {
        ui.label(RichText::new("Races: detecting from Bob's shop…").color(draw::GRAY));
        return;
    };
    ui.horizontal_wrapped(|ui| {
        for r in races {
            draw::pill(ui, pool::race_label(r), draw::hex("2f5d3a"), Color32::WHITE);
        }
        let n = owlingstone::season::get().lobby_tribes;
        if races.len() < n {
            let maybe: Vec<&str> = s.possible_races.iter().map(|r| pool::race_label(r)).collect();
            let extra = if maybe.is_empty() { String::new() } else { format!(" (maybe {})", maybe.join(", ")) };
            ui.label(RichText::new(format!("+{} unknown{extra}", n - races.len())).color(draw::GRAY));
        }
    });
}

/// The comp text: shown here only when the right panel is off (it has the same, with pictures).
fn comp_text(app: &App, ui: &mut egui::Ui) {
    let s = &app.snap;
    if !s.in_game {
        return;
    }
    let k = owlingstone::knowledge::get();
    let adv = &s.advice;
    let deity = |r: Rich| match &s.deity {
        Some(d) => r.gray(" · Deity: ").b(d),
        None => r,
    };
    if !adv.heroes.is_empty() {
        let mut r = Rich::new().gray("Pick (avg place): ");
        for (i, h) in adv.heroes.iter().enumerate() {
            if i > 0 {
                r = r.t(" · ");
            }
            match h.stat {
                Some(st) if i == 0 => r = r.bc(format!("{} {:.2}", h.name, st.avg), draw::GOLD),
                Some(st) => r = r.t(&h.name).gray(format!(" {:.2}", st.avg)),
                None => r = r.t(&h.name),
            }
        }
        deity(r).show(ui);
        return;
    }
    let buy = |mut r: Rich| {
        if !adv.buy.is_empty() {
            r = r.br().c("Buy: ", draw::SOFT_GREEN);
            for (i, p) in adv.buy.iter().enumerate() {
                if i > 0 {
                    r = r.t(", ");
                }
                r = r.t(&p.card.name).gray(format!(" ({})", p.reasons.join(", ")));
            }
        }
        r
    };
    match &adv.playing {
        None => {
            let names: Vec<&str> = adv.lobby.iter().filter_map(|id| k.comp(id)).map(|c| c.name.as_str()).collect();
            let mut text = names.iter().take(LOBBY_SHOWN).copied().collect::<Vec<_>>().join(", ");
            if names.len() > LOBBY_SHOWN {
                text += &format!(" (+{} more)", names.len() - LOBBY_SHOWN);
            }
            buy(deity(Rich::new().gray("Lobby comps: ").t(text))).show(ui);
        }
        Some(best) => {
            let mut r = Rich::new().t("Playing: ").bc(&best.name, draw::GOLD).t(format!(" ({}%)", best.confidence));
            r = deity(r).br().gray("Have: ");
            let have: Vec<&str> = best.have.iter().map(|c| c.name.as_str()).collect();
            r = r.t(if have.is_empty() { "–".to_string() } else { have.join(", ") }).br().gray("Need: ");
            let need: Vec<String> = best
                .missing_core
                .iter()
                .take(3)
                .map(|c| format!("{} (T{})", c.name, c.tier.map(|t| t.to_string()).unwrap_or_default()))
                .collect();
            r = r.t(if need.is_empty() { "all core cards ✓".to_string() } else { need.join(", ") });
            if let Some(alt) = &adv.alt {
                r = r.br().gray(format!("or {} ({}%)", alt.name, alt.confidence));
            }
            let resp = buy(r).show(ui);
            if let Some(comp) = k.comp(&best.comp_id) {
                let mut tip = vec![comp.when_to_commit.clone(), comp.how_to_play.clone()];
                tip.extend(best.notes.iter().cloned());
                resp.on_hover_text(tip.join("\n"));
            }
        }
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui, hover: &mut Option<Hover>) {
    let ctx = &ui.ctx().clone();
    let panel = egui::Frame::NONE.fill(ctx.global_style().visuals.panel_fill).inner_margin(8.0).corner_radius(4);
    egui::CentralPanel::default().frame(panel).show(ui, |ui| {
        let snap = app.snap.clone();
        let s = &*snap;
        header(ui, s);
        races_line(ui, s);
        for e in &s.data_errors {
            ui.label(RichText::new(e).color(draw::RED));
        }
        if !app.settings.lobby_panel() {
            comp_text(app, ui);
        }
        // Tier buttons (7 greyed while not reachable in this lobby, but clickable).
        let db = cards::db();
        let subsets = pool::load_subsets_cached();
        let sure = s.sure_races();
        let t7 = pool::tier7_reachable(&db, sure, &subsets, s.duos, app.seen_tier7 || s.tier.unwrap_or(0) >= 7);
        let shown = app.manual_tier.or(s.tier).unwrap_or(1);
        ui.add_space(2.0);
        ui.columns(7, |cols| {
            for (i, col) in cols.iter_mut().enumerate() {
                let t = i as i64 + 1;
                let mut text = RichText::new(t.to_string()).strong();
                if t == 7 && !t7 {
                    text = text.color(draw::DARK_GRAY);
                }
                if t == shown {
                    text = text.color(Color32::from_rgb(0x11, 0x11, 0x11));
                }
                let mut b = egui::Button::new(text).min_size(egui::vec2(col.available_width(), 24.0));
                if t == shown {
                    b = b.fill(draw::GOLD);
                }
                if Some(t) == s.tier && t != shown {
                    b = b.stroke(egui::Stroke::new(2.0, draw::GOLD));
                }
                let mut resp = col.add(b);
                if t == 7 && !t7 {
                    resp = resp.on_hover_text(
                        "Tier 7 not reachable yet: no Greedy Conniver (Pirates) in this lobby, and no Tier 7 card seen. Click to look anyway.",
                    );
                }
                if resp.clicked() {
                    // Clicking my own tier (or the chosen one again) = follow my tier again.
                    app.manual_tier = if Some(t) == s.tier || Some(t) == app.manual_tier { None } else { Some(t) };
                }
            }
        });
        // Keyword filter (Deathrattle, Battlecry...): only those possible in this lobby.
        let keywords = pool::available_keywords(&db, sure, &subsets, s.duos);
        if app.keyword.is_some_and(|k| !keywords.contains(&k)) {
            app.keyword = None;
        }
        egui::ComboBox::from_id_salt("keyword")
            .width(ui.available_width() - 8.0)
            .selected_text(app.keyword.unwrap_or("All keywords"))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut app.keyword, None, "All keywords");
                for k in &keywords {
                    ui.selectable_value(&mut app.keyword, Some(*k), *k);
                }
            });
        // Bottom rows first (the list takes what is left).
        egui::Panel::bottom("left-bottom").frame(egui::Frame::NONE).show(ui, |ui| bottom(app, ui));
        let star = app.shown_comp().and_then(|id| owlingstone::knowledge::get().comp(&id).map(|c| {
            c.core.iter().chain(&c.addons).map(|x| x.id.clone()).collect::<std::collections::HashSet<_>>()
        }));
        let star = star.unwrap_or_default();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (section, list) in pool::tier_list(&db, shown, sure, &subsets, s.duos, app.keyword) {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{}  ({})", pool::race_label(&section), list.len())).font(draw::bold(12.0)).color(draw::GOLD));
                for c in &list {
                    let mut mark = String::new();
                    if s.advice.buy_cards.contains(&c.id) {
                        mark.push('▶');
                    }
                    if star.contains(&c.id) {
                        mark.push('★');
                    }
                    let state = if s.shop.contains(&c.id) {
                        State::Shop
                    } else if s.board.contains(&c.id) {
                        State::Board
                    } else if s.hand.contains(&c.id) {
                        State::Hand
                    } else {
                        State::None
                    };
                    card_tile(app, c, state, &mark).show(ui, hover);
                }
            }
        });
    });
}

fn bottom(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let mut hide = app.settings.hide_when_unfocused();
    if ui.checkbox(&mut hide, "Auto-hide when Hearthstone is not focused").changed() {
        app.settings.set_flag("hide_when_unfocused", hide);
        app.save_settings();
    }
    ui.horizontal(|ui| {
        let mut art = app.settings.card_art();
        if ui.checkbox(&mut art, "Card art").changed() {
            app.settings.set_flag("card_art", art);
            app.save_settings();
        }
        let mut lobby = app.settings.lobby_panel();
        if ui
            .checkbox(&mut lobby, "Right panel")
            .on_hover_text("Panel on the right of the game. Comps tab: what I play, what to buy, every comp of this lobby. Lobby tab: next opponent's last board, the 8 players.")
            .changed()
        {
            app.settings.set_flag("lobby_panel", lobby);
            app.save_settings();
        }
    });
    // On the version's line: the line above has no room left in a narrow panel.
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("v{}", owlingstone::VERSION)).color(draw::GRAY));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("My stats").clicked() {
                app.dialogs.stats = Some(String::new());
            }
        });
    });
    if let Some(v) = app.update_ready.clone() {
        let b = egui::Button::new(RichText::new(format!("Update to v{v}")).strong().color(Color32::from_rgb(0x11, 0x11, 0x11)))
            .fill(draw::GOLD)
            .min_size(egui::vec2(ui.available_width(), 0.0));
        if ui
            .add(b)
            .on_hover_text("Installs the new version (checked and signed), then restarts Owlingstone. Your history and settings are kept.")
            .clicked()
        {
            app.run_update();
        }
    }
}
