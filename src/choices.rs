//! Advice on the choice the game shows now: the hero pick, the Trinket offer, a Discover (Triple reward,
//! spells, hero powers, minions that Discover...). Each option gets a chance (in %) to be the best pick
//! and a few words on why; the best one is marked.
//!
//! - What is known: the options themselves (Power.log names them, see `Game::open_choice`), my board and
//!   hand, my tier and gold, the lobby's tribes, the comps they can make, my next opponent's expected
//!   board (planner.rs), and the stats of heroes and Trinkets (data/heroes.yaml, data/trinkets.yaml).
//! - How a score is made: heroes and Trinkets by expected place (lower = better, the same ranking as the
//!   Comps tab); minions and spells by points like the Buy advice: comp core / add-on, triple or pair,
//!   stats added, economy, synergy with the board now, and the change in my next fight (simulated).
//! - Scores become chances with a "softmax": the gap between two options, divided by how much the score
//!   can be off (`*_SPREAD`), says how much likelier one is to be the best. It is an educated guess, not
//!   a certainty: options without data share the chance evenly and the advice says so.

use std::sync::Arc;

use crate::advice::{self, ShopPick};
use crate::cards::Card;
use crate::knowledge::Comp;
use crate::knowledge::Knowledge;
use crate::planner::{FIGHT_WEIGHT, OptionFights, board_synergy, board_value, economy_value, fight_value};
use crate::snapshot::HeroOption;
use crate::trinkets::TrinketPick;

/// How far off an expected place can be (heroes and Trinkets): 0.25 place better = e times likelier.
pub const PLACE_SPREAD: f64 = 0.25;
/// How far off a points score can be (minions and spells, a comp core card = 3 points).
pub const POINTS_SPREAD: f64 = 1.5;
/// Expected place of a hero without stats.
const UNKNOWN_HERO_AVG: f64 = 4.5;
/// A Trinket I cannot pay now: counted this much worse.
const UNAFFORDABLE: f64 = 1.0;
/// Points per tier for a Tavern spell without other data.
const SPELL_TIER_POINTS: f64 = 0.4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Hero,
    Trinket,
    Discover,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OptionAdvice {
    pub card_id: String,
    pub name: String,
    /// Chance (0-100) that this is the best pick.
    pub chance: f64,
    /// The score in short, e.g. "avg 3.85" or "4.2 pts".
    pub score: String,
    pub reasons: Vec<String>,
    pub best: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceAdvice {
    pub kind: Kind,
    /// e.g. "Hero pick", "Trinket offer", "Discover · Triple Reward".
    pub title: String,
    /// In the order the game shows them (left to right).
    pub options: Vec<OptionAdvice>,
    /// False when no option has data: the chances are even, a pure guess.
    pub sure: bool,
}

/// Chances (0-100, summing to 100) that each option is the best, from scores (higher = better) and how
/// far off a score can be. Equal scores share evenly.
pub fn chances(scores: &[f64], spread: f64) -> Vec<f64> {
    if scores.is_empty() {
        return vec![];
    }
    let top = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let w: Vec<f64> = scores.iter().map(|s| ((s - top) / spread.max(1e-6)).exp()).collect();
    let sum: f64 = w.iter().sum();
    w.iter().map(|x| 100.0 * x / sum).collect()
}

/// Marks the best option (the first of the highest chance) and checks there is something to say.
fn finish(kind: Kind, title: String, mut options: Vec<OptionAdvice>, sure: bool) -> Option<ChoiceAdvice> {
    if options.is_empty() {
        return None;
    }
    if sure {
        let best = (0..options.len()).fold(0, |b, i| if options[i].chance > options[b].chance + 1e-9 { i } else { b });
        options[best].best = true;
    }
    Some(ChoiceAdvice { kind, title, options, sure })
}

/// Hero pick: `ranked` (advice.rs order, best first) shown in `screen` order (card ids left to right).
pub fn for_heroes(ranked: &[HeroOption], screen: &[String]) -> Option<ChoiceAdvice> {
    let shown: Vec<&HeroOption> = screen.iter().filter_map(|id| ranked.iter().find(|h| h.card_id == *id)).collect();
    let shown = if shown.len() == ranked.len() { shown } else { ranked.iter().collect() };
    let avg = |h: &HeroOption| h.stat.as_ref().map(|s| s.avg).unwrap_or(UNKNOWN_HERO_AVG);
    let odds = chances(&shown.iter().map(|h| -avg(h)).collect::<Vec<_>>(), PLACE_SPREAD);
    let sure = shown.iter().any(|h| h.stat.is_some());
    let options = shown
        .iter()
        .zip(odds)
        .map(|(h, chance)| {
            let mut reasons = Vec::new();
            if let Some((comp, a, _)) = h.best_comps.first() {
                reasons.push(format!("best with {comp} ({a:.2})"));
            }
            if !h.note.is_empty() {
                reasons.push(h.note.clone());
            }
            OptionAdvice {
                card_id: h.card_id.clone(),
                name: h.name.clone(),
                chance,
                score: match &h.stat {
                    Some(s) => format!("avg {:.2}", s.avg),
                    None => "no stats".into(),
                },
                reasons,
                best: false,
            }
        })
        .collect();
    finish(Kind::Hero, "Hero pick".into(), options, sure)
}

/// Trinket offer: `ranked` (trinkets.rs, best first) shown in `screen` order (card ids left to right).
pub fn for_trinkets(ranked: &[TrinketPick], screen: &[String]) -> Option<ChoiceAdvice> {
    let mut shown: Vec<&TrinketPick> = Vec::new();
    for id in screen {
        if let Some(p) = ranked.iter().find(|p| p.card.id == *id && !shown.iter().any(|s| std::ptr::eq(*s, *p))) {
            shown.push(p);
        }
    }
    if shown.len() != ranked.len() {
        shown = ranked.iter().collect();
    }
    let score = |p: &TrinketPick| p.score + if p.affordable { 0.0 } else { UNAFFORDABLE };
    let odds = chances(&shown.iter().map(|p| -score(p)).collect::<Vec<_>>(), PLACE_SPREAD);
    let sure = shown.iter().any(|p| p.stat.is_some());
    let options = shown
        .iter()
        .zip(odds)
        .map(|(p, chance)| OptionAdvice {
            card_id: p.card.id.clone(),
            name: p.card.name.clone(),
            chance,
            score: format!("{:.2}", p.score),
            reasons: p.reasons.iter().filter(|r| !r.starts_with("avg ")).take(3).cloned().collect(),
            best: false,
        })
        .collect();
    finish(Kind::Trinket, "Trinket offer · expected place".into(), options, sure)
}

/// What a Discover option is worth, in points (a comp core card = 3), and why.
#[allow(clippy::too_many_arguments)]
fn option_points(
    k: &Knowledge,
    card: &Card,
    i: usize,
    comp: Option<&ShopPick>,
    team: &[Arc<Card>],
    fights: Option<&OptionFights>,
    turn: i64,
    tier: i64,
) -> (f64, Vec<String>, bool) {
    let mut value = 0.0;
    let mut why: Vec<String> = Vec::new();
    let mut known = false;
    if let Some(p) = comp {
        value += p.value;
        why.extend(p.reasons.iter().cloned());
        known = true;
    }
    let (econ, econ_why) = economy_value(k, &card.id, turn);
    if econ != 0.0 {
        value += econ;
        why.push(econ_why);
        known = true;
    }
    let (syn, syn_why) = board_synergy(card, team);
    if syn != 0.0 {
        value += syn;
        why.push(syn_why);
        known = true;
    }
    let minion = card.is_pool_minion || (card.attack + card.health > 0 && !card.is_pool_spell);
    if minion {
        known = true;
        let gain = fights.and_then(|f| f.with.get(i)).map(|w| w.1).unwrap_or(card.attack + card.health);
        let stats = board_value(gain, turn, tier);
        value += stats;
        if let Some(f) = fights {
            let (s, _) = &f.with[i];
            let delta = fight_value(s, f.health) - fight_value(&f.base, f.health);
            value += FIGHT_WEIGHT * delta;
            let gain = s.win - f.base.win;
            if gain >= 1.0 {
                why.push(format!("+{gain:.0}% win next fight"));
            } else if stats >= 0.5 {
                why.push("stats for later fights".into());
            }
        } else if stats >= 0.5 {
            why.push("good stats now".into());
        }
    } else if card.is_pool_spell {
        known = true;
        let t = card.tier.unwrap_or(1);
        value += SPELL_TIER_POINTS * t as f64;
        if why.is_empty() {
            why.push(format!("Tier {t} spell"));
        }
    }
    if !known {
        why.push("no data".into());
    }
    (value, why, known)
}

/// A Discover (or any other pick of cards): ranks the options by points.
///
/// options: cards in screen order; board / hand: my cards; owned: my non-golden cards (triples);
/// targets: comps aimed for (advice::target_comps); fights: next fight with each option (planner).
#[allow(clippy::too_many_arguments)]
pub fn for_discover(
    k: &Knowledge,
    source: Option<&str>,
    options: &[Arc<Card>],
    board: &[Arc<Card>],
    hand: &[Arc<Card>],
    owned: &[Arc<Card>],
    targets: &[(&Comp, f64)],
    fights: Option<&OptionFights>,
    turn: i64,
    tier: i64,
) -> Option<ChoiceAdvice> {
    let comp_picks = advice::shop_advice(options, owned, targets);
    let team: Vec<Arc<Card>> = board.iter().chain(hand).cloned().collect();
    let scored: Vec<(f64, Vec<String>, bool)> = options
        .iter()
        .enumerate()
        .map(|(i, c)| option_points(k, c, i, comp_picks.iter().find(|p| p.card.id == c.id), &team, fights, turn, tier))
        .collect();
    let sure = scored.iter().any(|s| s.2);
    let odds = chances(&scored.iter().map(|s| s.0).collect::<Vec<_>>(), POINTS_SPREAD);
    let opts = options
        .iter()
        .zip(scored)
        .zip(odds)
        .map(|((c, (v, why, _)), chance)| OptionAdvice {
            card_id: c.id.clone(),
            name: c.name.clone(),
            chance,
            score: format!("{v:.1} pts"),
            reasons: why.into_iter().take(3).collect(),
            best: false,
        })
        .collect();
    // "Battlegrounds Dark Gift [DNT]" -> "Dark Gift": no internal tags or mode prefix.
    let source = source.map(|s| {
        let s = s.split(" [").next().unwrap_or(s).trim();
        s.strip_prefix("Battlegrounds ").unwrap_or(s).to_string()
    });
    let title = match source.as_deref().filter(|s| !s.is_empty()) {
        Some(s) => format!("Discover · {s}"),
        None => "Discover".into(),
    };
    finish(Kind::Discover, title, opts, sure)
}

/// Used by tests and the console: the option names, best first, with their chance.
pub fn describe(c: &ChoiceAdvice) -> String {
    let mut o: Vec<&OptionAdvice> = c.options.iter().collect();
    o.sort_by(|a, b| b.chance.partial_cmp(&a.chance).unwrap_or(std::cmp::Ordering::Equal));
    o.iter().map(|x| format!("{} {:.0}%", x.name, x.chance)).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests;
