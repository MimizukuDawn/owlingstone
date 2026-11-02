//! Trinkets: which one to take from the offer, and how to steer the next offer with my board.
//!
//! Offer rules (Blizzard's "Updated Trinket Rules" + wiki, still true in season 14):
//! - 2 offers per game, at the start of turn 6 (Lesser) and turn 9 (Greater): 4 Trinkets, buy one (its
//!   gold is paid that turn, so less gold for minions).
//! - The offer depends on the game's tribes, my board, my hand and my hero power.
//! - A "typed" Trinket (tied to a tribe) needs me to be "in" that tribe: at least 2 minions of it on turn
//!   6, 3 on turn 9. 3 different tribes = "menagerie".
//! - Each offer has: a Trinket of my most present tribe, at least one without tribe and at least one at 2
//!   gold or less. At most one of a tribe I'm not in: that one costs 2 less ("pivot").
//!   => The board (and hand) at the END of turn 5 / turn 8 decides the offer: `next_offer_hint` says
//!   what to keep to aim for a tribe.
//!
//! Choice (`rank_offer`): start from the average place of players who took the Trinket (bgbuddy.gg,
//! current patch, data/trinkets.yaml). That figure comes from players who took it when it suited them:
//! it is corrected when it does NOT suit me (tribe missing from my board and comps, mechanic missing),
//! and a bit when it suits me very well (minion given is a card of my comp, tribe of my comp, synergy).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use regex::Regex;

use crate::cards::Card;
use crate::knowledge::{Comp, Knowledge, TrinketStat};
use crate::planner::{RACE_WORDS, board_synergy, mentions_race, race_word};

/// Turn of the offer -> kind of Trinket.
pub fn offer_kind(turn: i64) -> Option<&'static str> {
    match turn {
        6 => Some("lesser"),
        9 => Some("greater"),
        _ => None,
    }
}

/// Minions of a tribe needed to be "in" it.
pub fn in_type(kind: &str) -> i64 {
    if kind == "lesser" { 2 } else { 3 }
}

/// Small samples are pulled towards an average Trinket.
const PRIOR_AVG: f64 = 3.95;
const PRIOR_GAMES: f64 = 500.0;
/// A Trinket missing from the stats.
const UNKNOWN_AVG: f64 = 4.0;

// Corrections, in places (more = worse). The stats already mean "when it suits": penalties are
// stronger than bonuses.
const OFF_TYPE: f64 = 0.45; // Trinket's tribe: none of mine, not in my comps
const WEAK_TYPE: f64 = 0.15; // tribe: only 1 minion (or only in the shop)
const COMP_TYPE: f64 = -0.10; // tribe of a comp I aim for
const GIVES_CORE: f64 = -0.30; // the minion given is a core card of my comp
const GIVES_ADDON: f64 = -0.12; // ... an add-on
const GIVES_OFF: f64 = 0.25; // the minion given is of a tribe I don't play at all
const NO_MECHANIC: f64 = 0.30; // needs a mechanic (Deathrattle, Divine Shield...) I don't have
const SYNERGY_STEP: f64 = -0.05; // per point of board_synergy (0.6 per minion that fits)
const SYNERGY_CAP: f64 = -0.15;
const PIVOT_GOLD: f64 = -0.04; // per gold saved (pivot -2)

/// Mechanic a Trinket's text asks for -> what a minion must have to profit (case-insensitive).
/// (No "Tavern spell": everybody buys them, and Blood Gems are spells.)
static NEEDS: LazyLock<Vec<(&'static str, Regex, Regex)>> = LazyLock::new(|| {
    let ci = |p: &str| Regex::new(&format!("(?i){p}")).expect("valid regex");
    [
        ("Deathrattle", r"\byour Deathrattle|Deathrattle minions?|your minions?' Deathrattles|friendly Deathrattle", r"Deathrattle"),
        ("Divine Shield", r"(your|friendly)[^.]*Divine Shield|minions? with Divine Shield", r"Divine Shield"),
        ("Battlecry", r"\bBattlecry minions?|your Battlecr|Battlecries", r"^(<b>)?Battlecry|\bBattlecry:"),
        ("Start of Combat", r"your Start of Combat|Start of Combat minions?", r"Start of Combat"),
        ("End of Turn", r"your End of Turn|End of Turn minions?", r"end of (your )?turn"),
        ("Avenge", r"\bAvenge minions?|your Avenge", r"Avenge"),
        ("Rally", r"\bRally minions?|your Rally", r"Rally"),
        ("Magnetic", r"Magnetize", r"Magnetic"),
        ("Blood Gem", r"Blood Gems?", r"Blood Gem"),
    ]
    .into_iter()
    .map(|(n, w, h)| (n, ci(w), ci(h)))
    .collect()
});
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>|\[x\]").expect("valid regex"));

fn clean(text: &str) -> String {
    TAGS.replace_all(text, "").replace('\n', " ")
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrinketPick {
    pub card: Arc<Card>,
    /// Real price (after the "pivot" discount).
    pub cost: i64,
    /// Expected place (lower = better).
    pub score: f64,
    pub stat: Option<TrinketStat>,
    pub reasons: Vec<String>,
    pub affordable: bool,
}

/// Minions per tribe, in the order first seen (an Amalgam counts for all, a dual tribe for both).
pub fn race_counts(cards: &[Arc<Card>]) -> Vec<(String, i64)> {
    let mut n: Vec<(String, i64)> = Vec::new();
    for c in cards {
        if c.tier.is_none_or(|t| t == 0) {
            continue;
        }
        let races: Vec<String> =
            if c.has_race("ALL") { RACE_WORDS.iter().map(|(r, _)| r.to_string()).collect() } else { c.races.clone() };
        for r in races {
            match n.iter_mut().find(|(x, _)| *x == r) {
                Some(slot) => slot.1 += 1,
                None => n.push((r, 1)),
            }
        }
    }
    n
}

fn count(counts: &[(String, i64)], race: &str) -> i64 {
    counts.iter().find(|(r, _)| r == race).map(|(_, n)| *n).unwrap_or(0)
}

fn base_avg(stat: Option<&TrinketStat>) -> f64 {
    let Some(s) = stat else { return UNKNOWN_AVG };
    if s.games < 5000.0 { (s.avg * s.games + PRIOR_AVG * PRIOR_GAMES) / (s.games + PRIOR_GAMES) } else { s.avg }
}

/// Tribes tied to the Trinket: the card's, else the given minion's, else named in its text.
pub fn trinket_races(k: &Knowledge, card: &Card, stat: Option<&TrinketStat>) -> Vec<String> {
    if let Some(s) = stat {
        if !s.races.is_empty() {
            return s.races.iter().filter(|r| race_word(r).is_some()).cloned().collect();
        }
        if let Some(given) = s.gives.as_ref().and_then(|g| k.minion_by_name.get(g)) {
            if !given.races.is_empty() && !given.has_race("ALL") {
                return given.races.clone();
            }
        }
    }
    let text = clean(&card.text);
    RACE_WORDS.iter().map(|(r, _)| *r).filter(|r| *r != "NAGA" && mentions_race(&text, r)).map(str::to_string).collect()
}

/// Ranks the offered Trinkets, best first.
///
/// offer: [(card, real price)]; board / hand / shop: cards; targets: [(comp, weight)] (target_comps);
/// powers: my hero powers' cards; gold: my gold (None = unknown).
#[allow(clippy::too_many_arguments)]
pub fn rank_offer(
    k: &Knowledge,
    offer: &[(Arc<Card>, i64)],
    board: &[Arc<Card>],
    hand: &[Arc<Card>],
    shop: &[Arc<Card>],
    targets: &[(&Comp, f64)],
    powers: &[Arc<Card>],
    gold: Option<i64>,
) -> Vec<TrinketPick> {
    let mine: Vec<Arc<Card>> = board.iter().chain(hand).cloned().collect();
    let counts = race_counts(&mine);
    let shop_counts = race_counts(shop);
    let main_targets: Vec<&Comp> = targets.iter().filter(|(_, w)| *w >= 0.5).map(|(c, _)| *c).collect();
    let comp_races: Vec<&String> = main_targets.iter().flat_map(|c| &c.races).collect();
    let core: HashMap<&str, &str> =
        main_targets.iter().flat_map(|comp| comp.core.iter().map(move |c| (c.name.as_str(), comp.name.as_str()))).collect();
    let addons: HashMap<&str, &str> =
        main_targets.iter().flat_map(|comp| comp.addons.iter().map(move |c| (c.name.as_str(), comp.name.as_str()))).collect();
    let mine_and_powers: Vec<Arc<Card>> = mine.iter().chain(powers).cloned().collect();
    let mut picks = Vec::new();
    for (card, cost) in offer {
        let stat = k.trinkets.get(&card.id);
        let mut score = base_avg(stat);
        let mut why = Vec::new();
        match stat {
            Some(s) => why.push(format!("avg {:.2}{}", s.avg, if s.games < 500.0 { " (few games)" } else { "" })),
            None => why.push("no stats".to_string()),
        }
        let races = trinket_races(k, card, stat);
        if !races.is_empty() {
            let have = races.iter().map(|r| count(&counts, r)).max().unwrap_or(0);
            let word = races.iter().map(|r| race_word(r).unwrap_or(r)).collect::<Vec<_>>().join("/");
            if races.iter().any(|r| comp_races.contains(&r)) {
                score += COMP_TYPE;
                why.push(format!("{word}: your comp"));
            } else if have >= 2 {
                why.push(format!("{word}: {have} on board"));
            } else if have == 1 || races.iter().any(|r| count(&shop_counts, r) > 0) {
                score += WEAK_TYPE;
                why.push(if have > 0 { format!("{word}: only {have} on board") } else { format!("{word}: none yet (some in shop)") });
            } else {
                score += OFF_TYPE;
                why.push(format!("no {word} — pivot only"));
            }
        }
        let gives = stat.and_then(|s| s.gives.as_deref());
        if let Some(g) = gives {
            if let Some(comp) = core.get(g) {
                score += GIVES_CORE;
                why.push(format!("gives {g} (core of {comp})"));
            } else if let Some(comp) = addons.get(g) {
                score += GIVES_ADDON;
                why.push(format!("gives {g} ({comp})"));
            } else if !races.is_empty()
                && !races.iter().any(|r| count(&counts, r) > 0)
                && !races.iter().any(|r| comp_races.contains(&r))
            {
                score += GIVES_OFF;
            }
        }
        let text = clean(&card.text);
        for (name, wants, has) in NEEDS.iter() {
            if wants.is_match(&text) && !mine_and_powers.iter().any(|c| has.is_match(&clean(&c.text))) {
                if *name == "Blood Gem" && count(&counts, "QUILBOAR") > 0 {
                    continue;
                }
                score += NO_MECHANIC;
                why.push(format!("no {name} on board"));
                break;
            }
        }
        let (pts, syn) = board_synergy(card, &mine_and_powers);
        if pts != 0.0 {
            score += SYNERGY_CAP.max(SYNERGY_STEP * pts);
            why.push(syn);
        }
        let base_cost = stat.map(|s| s.cost).unwrap_or(card.cost);
        if base_cost != 0 && *cost < base_cost {
            score += PIVOT_GOLD * (base_cost - cost) as f64;
            why.push(format!("pivot: {cost} gold instead of {base_cost}"));
        }
        let affordable = gold.is_none_or(|g| *cost <= g);
        if !affordable {
            why.push(format!("costs {cost}, you have {}", gold.unwrap_or(0)));
        }
        picks.push(TrinketPick {
            card: card.clone(),
            cost: *cost,
            score: (score * 100.0).round() / 100.0,
            stat: stat.cloned(),
            reasons: why,
            affordable,
        });
    }
    picks.sort_by(|a, b| (!a.affordable, a.score).partial_cmp(&(!b.affordable, b.score)).unwrap_or(std::cmp::Ordering::Equal));
    picks
}

#[derive(Debug, Clone, PartialEq)]
pub struct OfferHint {
    pub kind: &'static str,
    pub need: i64,
    pub lines: Vec<String>,
    pub unlocked: Vec<String>,
    pub close: Vec<String>,
}

/// Before an offer (turns 5 and 8: the offer comes at the start of the next turn): which tribes my board
/// + hand unlock, and what to add to unlock another. None on other turns.
pub fn next_offer_hint(
    k: &Knowledge,
    turn: i64,
    board: &[Arc<Card>],
    hand: &[Arc<Card>],
    lobby_races: Option<&[String]>,
    targets: &[(&Comp, f64)],
) -> Option<OfferHint> {
    const TOP: usize = 2;
    let kind = offer_kind(turn + 1)?;
    let need = in_type(kind);
    let mine: Vec<Arc<Card>> = board.iter().chain(hand).cloned().collect();
    let counts = race_counts(&mine);
    let comp_races: Vec<&String> = targets.iter().filter(|(_, w)| *w >= 0.5).flat_map(|(c, _)| &c.races).collect();
    let in_lobby = |r: &str| match lobby_races {
        Some(l) if !l.is_empty() => l.iter().any(|x| x == r),
        _ => r != "NAGA",
    };
    let mut best: HashMap<&str, Vec<&TrinketStat>> = HashMap::new();
    for t in k.trinket_list.iter().filter(|t| t.kind == kind) {
        for r in &t.races {
            best.entry(r.as_str()).or_default().push(t);
        }
    }
    for list in best.values_mut() {
        list.sort_by(|a, b| base_avg(Some(a)).partial_cmp(&base_avg(Some(b))).unwrap_or(std::cmp::Ordering::Equal));
    }
    let names = |race: &str| {
        best.get(race)
            .map(|l| l.iter().take(TOP).map(|t| format!("{} {:.2}", t.name, t.avg)).collect::<Vec<_>>().join(", "))
            .unwrap_or_default()
    };
    let unlocked: Vec<String> =
        RACE_WORDS.iter().map(|(r, _)| *r).filter(|r| count(&counts, r) >= need && in_lobby(r)).map(str::to_string).collect();
    let mut close: Vec<String> = RACE_WORDS
        .iter()
        .map(|(r, _)| *r)
        .filter(|r| count(&counts, r) == need - 1 && in_lobby(r) && !unlocked.iter().any(|u| u == r))
        .map(str::to_string)
        .collect();
    let first_avg = |r: &str| best.get(r).and_then(|l| l.first()).map(|t| base_avg(Some(t))).unwrap_or(9.0);
    close.sort_by(|a, b| {
        (!comp_races.contains(&a), first_avg(a))
            .partial_cmp(&(!comp_races.contains(&b), first_avg(b)))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let title = if kind == "lesser" { "Lesser" } else { "Greater" };
    let mut lines =
        vec![format!("{title} Trinkets next turn: end this turn with {need}+ of a tribe (board + hand) to be offered its Trinkets.")];
    for r in &unlocked {
        let n = names(r);
        lines.push(format!("✓ {} ({}): {}", race_word(r).unwrap_or(r), count(&counts, r), if n.is_empty() { "no stats".into() } else { n }));
    }
    let close: Vec<String> = close.into_iter().filter(|r| best.get(r.as_str()).is_some_and(|l| !l.is_empty())).collect();
    for r in close.iter().take(2) {
        let tag = if comp_races.contains(&r) { " — your comp" } else { "" };
        lines.push(format!("+1 {} unlocks: {}{tag}", race_word(r).unwrap_or(r), names(r)));
    }
    if counts.iter().filter(|(_, n)| *n > 0).count() >= 3 {
        lines.push("3+ tribes: menagerie Trinkets possible too.".into());
    }
    // Main tribe: the first one with the highest count.
    let main = counts.iter().fold(None::<&(String, i64)>, |best, c| match best {
        Some(b) if b.1 >= c.1 => Some(b),
        _ => Some(c),
    });
    if let Some((r, n)) = main {
        if *n >= 1 {
            lines.push(format!("Guaranteed: one {} Trinket (your main tribe).", race_word(r).unwrap_or(r)));
        }
    }
    Some(OfferHint { kind, need, lines, unlocked, close: close.into_iter().take(2).collect() })
}

#[cfg(test)]
mod tests;
