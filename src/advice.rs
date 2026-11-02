//! Advice: which hero to pick, what to buy in the shop.
//!
//! - `rank_heroes(names)` sorts the offered heroes, best average place first.
//! - `target_comps(...)`: the comps I aim for. If my board already says what I play, the recognised
//!   comps; else every comp of the lobby, the best ones (my rules, then average place) weighing more.
//! - `non_golden(...)` removes my golden cards before counting triples.
//! - `shop_advice(...)` scores each shop minion: 3 if "core" of a target comp, 1 if an add-on (× the
//!   comp's weight), +3 if it makes a triple, +1 if it makes a pair.

use std::collections::HashMap;
use std::sync::Arc;

use crate::cards::Card;
use crate::comps::{self, ADDON_POINTS, CORE_POINTS};
use crate::knowledge::{Comp, HeroStat, Knowledge};
use crate::rules::Ctx;

pub const TRIPLE_POINTS: f64 = 3.0;
pub const PAIR_POINTS: f64 = 1.0;
/// Same threshold as the "Playing" panel.
pub const PLAYING_MIN_CONFIDENCE: i64 = 15;
/// At the start of a game: the 3 best comps of the lobby count more (nothing is decided yet).
const LOBBY_TOP: usize = 3;
const LOBBY_WEIGHTS: (f64, f64) = (0.5, 0.25);

/// [(name, stats)] best average place first; heroes without stats last.
pub fn rank_heroes(names: &[String], heroes: &HashMap<String, HeroStat>) -> Vec<(String, Option<HeroStat>)> {
    let mut out: Vec<(String, Option<HeroStat>)> = names.iter().map(|n| (n.clone(), heroes.get(n).copied())).collect();
    out.sort_by(|a, b| {
        let key = |x: &Option<HeroStat>| x.map(|s| s.avg).unwrap_or(9.0);
        key(&a.1).partial_cmp(&key(&b.1)).unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// [(comp, weight)]: the comps my buys should serve.
pub fn target_comps<'k>(k: &'k Knowledge, board: &[Arc<Card>], hand: &[Arc<Card>], ctx: &Ctx) -> Vec<(&'k Comp, f64)> {
    let playing = comps::match_comps(&k.comps, board, hand, &k.rules, ctx, 2, PLAYING_MIN_CONFIDENCE);
    if !playing.is_empty() {
        return playing.iter().enumerate().map(|(i, m)| (m.comp, if i == 0 { 1.0 } else { 0.5 })).collect();
    }
    comps::lobby_comps(k, ctx)
        .into_iter()
        .enumerate()
        .map(|(i, c)| (c, if i < LOBBY_TOP { LOBBY_WEIGHTS.0 } else { LOBBY_WEIGHTS.1 }))
        .collect()
}

/// My cards without the golden ones (a golden card no longer counts for a triple). `golden_ids` may
/// hold the same id several times: one per golden copy.
pub fn non_golden(cards: &[Arc<Card>], golden_ids: &[String]) -> Vec<Arc<Card>> {
    let mut golden = golden_ids.to_vec();
    let mut kept = Vec::new();
    for c in cards {
        match golden.iter().position(|g| *g == c.id) {
            Some(i) => {
                golden.remove(i);
            }
            None => kept.push(c.clone()),
        }
    }
    kept
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShopPick {
    pub card: Arc<Card>,
    pub value: f64,
    /// e.g. ["core Mech APM", "triple"]
    pub reasons: Vec<String>,
}

/// Shop minions worth it, best first. `owned`: my non-golden cards (board + hand), for triples.
pub fn shop_advice(shop: &[Arc<Card>], owned: &[Arc<Card>], targets: &[(&Comp, f64)]) -> Vec<ShopPick> {
    let mut copies: HashMap<&str, usize> = HashMap::new();
    for c in owned {
        *copies.entry(c.id.as_str()).or_insert(0) += 1;
    }
    let mut picks = Vec::new();
    for card in shop {
        let mut value = 0.0;
        let mut reasons = Vec::new();
        // Only the best "comp" reason: a core card of 2 comps doesn't count twice.
        let mut best = (0.0, String::new());
        for (comp, w) in targets {
            let found = if comp.has_core(&card.id) {
                (CORE_POINTS * w, format!("core {}", comp.name))
            } else if comp.has_addon(&card.id) {
                (ADDON_POINTS * w, format!("add-on {}", comp.name))
            } else {
                (0.0, String::new())
            };
            if found.0 > best.0 || (found.0 == best.0 && found.1 > best.1) {
                best = found;
            }
        }
        if best.0 > 0.0 {
            value += best.0;
            reasons.push(best.1);
        }
        match copies.get(card.id.as_str()).copied().unwrap_or(0) {
            n if n >= 2 => {
                value += TRIPLE_POINTS;
                reasons.push("triple".into());
            }
            1 => {
                value += PAIR_POINTS;
                reasons.push("pair".into());
            }
            _ => {}
        }
        if value != 0.0 {
            picks.push(ShopPick { card: card.clone(), value, reasons });
        }
    }
    picks.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));
    picks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cards, data};

    fn cards_named(names: &[&str]) -> Vec<Arc<Card>> {
        let db = cards::test_db();
        let pool: Vec<Arc<Card>> = db.pool_minions(false).into_iter().chain(db.pool_spells(false)).collect();
        names.iter().map(|n| pool.iter().find(|c| c.name == *n).unwrap().clone()).collect()
    }

    fn know() -> Knowledge {
        Knowledge::build(&cards::test_db(), &|n| data::read(n))
    }

    #[test]
    fn heroes_file_matches_card_names() {
        let db = cards::test_db();
        let k = know();
        assert!(k.heroes.len() > 100);
        for name in k.heroes.keys() {
            assert!(db.by_id.values().any(|c| c.is_hero && c.name == *name), "{name}");
        }
    }

    #[test]
    fn rank_heroes_best_first_unknown_last() {
        let mut heroes = HashMap::new();
        heroes.insert("A".to_string(), HeroStat { avg: 4.1, games: 1.0 });
        heroes.insert("B".to_string(), HeroStat { avg: 3.6, games: 1.0 });
        let names: Vec<String> = ["A", "X", "B"].iter().map(|s| s.to_string()).collect();
        let ranked: Vec<String> = rank_heroes(&names, &heroes).into_iter().map(|(n, _)| n).collect();
        assert_eq!(ranked, ["B", "A", "X"]);
    }

    #[test]
    fn shop_advice_core_addon_triple_pair() {
        let k = know();
        let apm = k.comp("mech-apm").unwrap();
        let shop = cards_named(&["Auto Reveille", "Drone Duplicator", "Annoy-o-Module", "Wrath Weaver", "Glambot"]);
        let owned = cards_named(&["Annoy-o-Module", "Annoy-o-Module", "Glambot"]);
        let picks = shop_advice(&shop, &owned, &[(apm, 1.0)]);
        let names: Vec<&str> = picks.iter().map(|p| p.card.name.as_str()).collect();
        assert_eq!(names, ["Auto Reveille", "Annoy-o-Module", "Drone Duplicator", "Glambot"]);
        assert_eq!(picks[0].reasons, ["core Mech APM"]);
        assert_eq!(picks[1].reasons, ["triple"]);
        assert_eq!(picks[3].reasons, ["pair"]);
    }

    #[test]
    fn golden_copies_do_not_make_triples() {
        let two = cards_named(&["Annoy-o-Module", "Annoy-o-Module"]);
        let owned = non_golden(&two, &[two[0].id.clone()]);
        assert_eq!(owned.len(), 1);
        assert_eq!(shop_advice(&cards_named(&["Annoy-o-Module"]), &owned, &[])[0].reasons, ["pair"]);
    }

    #[test]
    fn target_comps_lobby_then_playing() {
        let k = know();
        let lobby: Vec<String> =
            ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"].iter().map(|s| s.to_string()).collect();
        let ctx = Ctx { races: Some(lobby), ..Default::default() };
        let early = target_comps(&k, &[], &[], &ctx);
        assert_eq!(early[..4].iter().map(|(_, w)| *w).collect::<Vec<_>>(), [0.5, 0.5, 0.5, 0.25]);
        let later = target_comps(&k, &cards_named(&["Auto Reveille", "Spark Snapper"]), &[], &ctx);
        assert_eq!((later[0].0.id.as_str(), later[0].1), ("mech-apm", 1.0));
    }
}
