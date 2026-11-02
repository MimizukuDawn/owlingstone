//! Comps: guess "what I'm playing" from my board, rank the comps of the lobby.
//!
//! - `available_comps(races)` keeps the comps whose tribes are all in the game (like HDT, which only
//!   offers the guides playable with the lobby's tribes).
//! - `match_comps(...)` scores each comp: 3 points per "core" card owned, 1 per "addon", plus a bonus
//!   for the share of the board in the right tribes. Returns the best ones, with the cards already owned
//!   and the missing ones.
//! - Rules (rules.rs) hide comps or add / remove points.
//! - `lobby_comps(...)`: comps offered at the start of a game, sorted by rules then average place.

use std::collections::HashMap;
use std::sync::Arc;

use crate::cards::Card;
use crate::knowledge::{Comp, Knowledge, Rule};
use crate::rules::{self, Ctx};

/// Keywords looked for in card texts to describe a board ("Deathrattle ×3").
pub const KEYWORDS: [&str; 15] = [
    "Deathrattle", "Battlecry", "Rally", "Spellcraft", "Magnetic", "Avenge", "Divine Shield", "Reborn", "Taunt",
    "Blood Gem", "Tavern spell", "Discard", "Deity", "Golden", "End of your turn",
];

pub const CORE_POINTS: f64 = 3.0;
pub const ADDON_POINTS: f64 = 1.0;
pub const RACE_BONUS: f64 = 4.0;
/// A card in hand counts half: the board says what I play.
pub const HAND_WEIGHT: f64 = 0.5;

#[derive(Debug, Clone)]
pub struct CompMatch<'k> {
    pub comp: &'k Comp,
    pub score: f64,
    /// 0-100: score / best possible score.
    pub confidence: i64,
    pub have: Vec<Arc<Card>>,
    pub missing_core: Vec<Arc<Card>>,
    pub missing_addons: Vec<Arc<Card>>,
    pub rule_bonus: f64,
    pub notes: Vec<String>,
}

/// Comps playable with these tribes (None: not detected yet -> all).
pub fn available_comps<'k>(comps: &'k [Comp], races: Option<&[String]>) -> Vec<&'k Comp> {
    comps.iter().filter(|c| races.is_none_or(|r| c.races.iter().all(|x| r.contains(x)))).collect()
}

/// Board summary: tribes and keywords, with counts.
pub fn board_profile(cards: &[Arc<Card>]) -> (HashMap<String, i64>, HashMap<&'static str, i64>) {
    let mut races = HashMap::new();
    let mut keywords = HashMap::new();
    for c in cards {
        for r in c.races.iter().filter(|r| *r != "ALL") {
            *races.entry(r.clone()).or_insert(0) += 1;
        }
        let text = c.text.to_lowercase();
        for k in KEYWORDS {
            if text.contains(&k.to_lowercase()) {
                *keywords.entry(k).or_insert(0) += 1;
            }
        }
    }
    (races, keywords)
}

/// Ranks the available comps by my cards and my rules; drops comps under `min_confidence` (%) before
/// keeping the `top` first.
pub fn match_comps<'k>(
    comps: &'k [Comp],
    board: &[Arc<Card>],
    hand: &[Arc<Card>],
    rules: &[Rule],
    ctx: &Ctx,
    top: usize,
    min_confidence: i64,
) -> Vec<CompMatch<'k>> {
    let mut weight: HashMap<&str, f64> = hand.iter().map(|c| (c.id.as_str(), HAND_WEIGHT)).collect();
    for c in board {
        weight.insert(c.id.as_str(), 1.0); // on the board: full weight
    }
    let minions: Vec<&Arc<Card>> = board.iter().filter(|c| !c.is_pool_spell).collect(); // the board's "colour"
    let mut results = Vec::new();
    for comp in available_comps(comps, ctx.races.as_deref()) {
        let verdict = rules::judge(&comp.id, rules, ctx);
        if verdict.hidden {
            continue;
        }
        let owned = |c: &Arc<Card>| weight.contains_key(c.id.as_str());
        let have: Vec<Arc<Card>> = comp.core.iter().chain(&comp.addons).filter(|c| owned(c)).cloned().collect();
        let hits = |cards: &[Arc<Card>]| cards.iter().map(|c| weight.get(c.id.as_str()).copied().unwrap_or(0.0)).sum::<f64>();
        let fit = if minions.is_empty() {
            0.0
        } else {
            let n = minions
                .iter()
                .filter(|c| c.races.iter().any(|r| comp.races.contains(r)) || c.races.iter().any(|r| r == "ALL"))
                .count();
            n as f64 / minions.len() as f64
        };
        let score = CORE_POINTS * hits(&comp.core) + ADDON_POINTS * hits(&comp.addons) + RACE_BONUS * fit;
        let best = CORE_POINTS * comp.core.len().min(3) as f64 + ADDON_POINTS * comp.addons.len().min(3) as f64 + RACE_BONUS;
        results.push(CompMatch {
            comp,
            score,
            confidence: ((100.0 * score / best).round_ties_even() as i64).min(100),
            have,
            missing_core: comp.core.iter().filter(|c| !owned(c)).cloned().collect(),
            missing_addons: comp.addons.iter().filter(|c| !owned(c)).cloned().collect(),
            rule_bonus: verdict.bonus,
            notes: verdict.notes,
        });
    }
    // What I play is what my board says: rules only break ties (they count fully in lobby_comps).
    // A comp with none of my cards is never "played": the share of right tribes alone is not enough.
    results.sort_by(|a, b| (b.score, b.rule_bonus).partial_cmp(&(a.score, a.rule_bonus)).unwrap_or(std::cmp::Ordering::Equal));
    results.into_iter().filter(|m| !m.have.is_empty() && m.confidence >= min_confidence).take(top).collect()
}

/// Average place of the comp: my hero's if it is one of the comp's best heroes, else the comp's
/// (9 without stats).
pub fn comp_avg(k: &Knowledge, comp: &Comp, hero: Option<&str>) -> f64 {
    hero.and_then(|h| k.hero_comps.get(&comp.id).and_then(|t| t.get(h)))
        .map(|(avg, _)| *avg)
        .or(comp.stats.avg)
        .unwrap_or(9.0)
}

/// Tier list: adjusted average place (my hero, my rules) -> letter. A 3-point prefer_if rule (= one
/// core card) gains 0.15 place; avoid_if loses as much.
pub const TIERS: [(&str, f64); 5] = [("S", 3.30), ("A", 3.50), ("B", 3.70), ("C", 3.90), ("D", 99.0)];
pub const RULE_PLACE: f64 = 0.05;

/// (letter, adjusted place) of the comp in this lobby; ("?", 9) without stats.
pub fn comp_tier(k: &Knowledge, comp: &Comp, hero: Option<&str>, bonus: f64) -> (&'static str, f64) {
    let avg = ((comp_avg(k, comp, hero) - RULE_PLACE * bonus) * 1e6).round() / 1e6; // 3.45 - 0.15 = 3.3
    if avg >= 9.0 {
        return ("?", avg);
    }
    (TIERS.iter().find(|(_, top)| avg <= *top).map(|(l, _)| *l).unwrap_or("D"), avg)
}

/// [(comp, the hero's average place with it, games)]: comps where this hero is among the best,
/// playable in this lobby (all if the tribes are unknown), best first.
pub fn hero_best_comps<'k>(k: &'k Knowledge, hero: &str, races: Option<&[String]>, top: usize) -> Vec<(&'k Comp, f64, f64)> {
    let mut found: Vec<(&Comp, f64, f64)> = available_comps(&k.comps, races)
        .into_iter()
        .filter_map(|c| k.hero_comps.get(&c.id).and_then(|t| t.get(hero)).map(|(a, g)| (c, *a, *g)))
        .collect();
    found.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    found.truncate(top);
    found
}

/// Comps playable in this lobby without those my rules hide: preferred first, then best average place
/// (my hero's when it is among the comp's best heroes; comps without stats last).
pub fn lobby_comps<'k>(k: &'k Knowledge, ctx: &Ctx) -> Vec<&'k Comp> {
    let mut judged: Vec<(&Comp, f64, f64)> = available_comps(&k.comps, ctx.races.as_deref())
        .into_iter()
        .map(|c| (c, rules::judge(&c.id, &k.rules, ctx)))
        .filter(|(_, v)| !v.hidden)
        .map(|(c, v)| (c, -v.bonus, comp_avg(k, c, ctx.hero.as_deref())))
        .collect();
    judged.sort_by(|a, b| (a.1, a.2).partial_cmp(&(b.1, b.2)).unwrap_or(std::cmp::Ordering::Equal));
    judged.into_iter().map(|(c, _, _)| c).collect()
}

#[cfg(test)]
mod tests;
