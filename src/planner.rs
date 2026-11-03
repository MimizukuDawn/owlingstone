//! Turn plan (recruit phase): level or not, what to buy, roll or freeze, given my health, my board's
//! strength and the expected board of my next opponent.
//!
//! - `expected_board(...)`: my next opponent's likely board NOW. Their last board seen is often 4 to 7
//!   turns old, and boards grow fast (×1.6 per turn on average): it is "aged" with the growth curve
//!   learned from real games (see GROWTH). Never fought = a typical board of the lobby at this turn.
//! - `fight_plan(game)`: runs in the log reader thread (like `combat_info`). Simulates my board (+ the
//!   minions in my hand, played for free) against that expected board: as is, with each shop minion
//!   added, with all my gold spent, and leveling with the gold left. Damage capped like in the game.
//! - `turn_plan(...)`: mixes these simulations with the comp points of advice.rs (core / add-on /
//!   triple) and the economy / card advantage of data/economy.yaml, and gives readable advice: Level or
//!   not, Buy, Sell, Roll, Freeze, and the danger for my health.
//!
//! Data: the growth curve comes from 13 Solo games of the current patch (150 fights). Per-turn stats of
//! Firestone / HSReplay are not reachable. The "when to level" curve follows the standard guide curve
//! (T2 on turn 2, T3 turn 5, T4 turn 7, T5 turn 9, T6 turn 12), shifted to the real pace of lobbies.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, LazyLock};

use regex::Regex;

use crate::cards::{self, Card, CardDb};
use crate::combat::{self, MinionSnap, SimInput, SimResult, snapshot_minion};
use crate::game::{Counters, Fight, Game};
use crate::knowledge::{Knowledge, PowerRule};

pub const MAX_BOARD: usize = 7;

/// log(attack + health of the opponent's board) = a + b × turn + c × opponent's tier.
/// Fitted on 150 fights (13 games, patch 36.6.3): residual standard deviation 0.73 (≈ ×2).
pub const GROWTH: (f64, f64, f64) = (0.71, 0.496, 0.154);
pub const GROWTH_SD: f64 = 0.73;
/// A stronger (or weaker) opponent stays so: keep 90% of their gap to the curve per 2 turns.
pub const PERSISTENCE: f64 = 0.9;
/// An old board "grown" stays too weak (new effects, shields, golden cards...): +0.5 in log (×1.65)
/// fits real results best.
pub const SEEN_BOOST: f64 = 0.5;
/// A typical board is made of minions without effects, and real boards of the same attack + health win
/// more often: +0.3 in log (×1.35) fits real results best (190 fights, 6 days of games).
pub const TYPICAL_BOOST: f64 = 0.3;
/// How far the real board's strength is from the expected one (standard deviation of the log of
/// attack + health): (never fought or seen before turn 4, old board grown). Measured: 0.5 and 0.85 on
/// 202 fights (patches 36.6.3 and 37.0.2); 0.7 and 1.0 (board makeup varies too) fit results best.
pub const BOARD_SD: (f64, f64) = (0.7, 1.0);
/// Strengths tried around the expected board, in standard deviations: the middles of 7 equally likely
/// slices of a normal curve.
const SPREAD: [f64; 7] = [-1.465, -0.792, -0.366, 0.0, 0.366, 0.792, 1.465];

/// Average number of minions on the opponent's board by turn.
fn board_count(turn: i64) -> usize {
    match turn {
        1 | 2 => 1,
        3 => 2,
        4 => 3,
        5 => 4,
        6 => 5,
        7 => 6,
        8 => 7,
        _ => MAX_BOARD,
    }
}

/// Tier aimed for at the END of each turn (standard curve, average lobby pace).
pub const LEVEL_CURVE: [(i64, i64); 12] =
    [(1, 1), (2, 2), (3, 2), (4, 2), (5, 3), (6, 3), (7, 4), (8, 4), (9, 5), (10, 5), (11, 5), (12, 6)];

/// Copies of each minion in the pool by tier.
fn pool_copies(tier: Option<i64>) -> f64 {
    match tier {
        Some(1) => 16.0,
        Some(2) => 15.0,
        Some(3) => 13.0,
        Some(4) => 11.0,
        Some(5) => 9.0,
        Some(6) => 7.0,
        Some(7) => 5.0,
        _ => 7.0,
    }
}

/// Shop size by my tier.
fn shop_slots(tier: i64) -> i32 {
    match tier {
        1 => 3,
        2 | 3 => 4,
        4 | 5 => 5,
        6 | 7 => 6,
        _ => 5,
    }
}

pub const MINION_COST: i64 = 3;
/// Points for a fight that goes from lost to won (a core card of a comp = 3).
pub const FIGHT_WEIGHT: f64 = 5.0;
/// Per board tried (about ten per shop).
pub const PLAN_RUNS: (u32, f64) = (1000, 0.08);

// ------------------------------------------------------------------------------------- math
/// Error function (Abramowitz and Stegun 7.1.26, error < 1.5e-7).
pub fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0
        - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t + 0.254_829_592)
            * t
            * (-x * x).exp();
    sign * y
}

// --------------------------------------------------------------------------------- hero profile
/// One of my hero powers: name, card id, current price, already used this turn.
#[derive(Debug, Clone, PartialEq)]
pub struct Power {
    pub name: String,
    pub card_id: String,
    pub cost: i64,
    pub exhausted: bool,
}

/// Plan settings for my hero: tier curve, gold kept for the hero power, preferences.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// Turn (1-20) -> tier to have at the end of it.
    pub curve: BTreeMap<i64, i64>,
    pub custom_curve: bool,
    /// Gold kept aside for the hero power.
    pub reserve: i64,
    pub power: Option<String>,
    pub level_bias: f64,
    pub minion_cost: i64,
    pub buy_bonus: BTreeMap<String, f64>,
    pub pair_bonus: f64,
    pub note: String,
    pub hero: Option<String>,
}

/// A hero missing from hero_curves.yaml: standard curve, and a 1-2 gold power is kept from turn 3.
pub fn hero_profile(k: &Knowledge, hero: Option<&str>, powers: &[Power], turn: i64) -> Profile {
    let entry = hero
        .and_then(|h| k.hero_curves.get(h))
        .or_else(|| powers.iter().find_map(|p| k.hero_curves.get(&p.name)));
    let marks: BTreeMap<i64, i64> = match entry {
        Some(e) if !e.curve.is_empty() => e.curve.clone(),
        _ => LEVEL_CURVE.iter().copied().collect(),
    };
    let mut curve = BTreeMap::new();
    let mut tier = 1;
    for t in 1..=20 {
        tier = marks.get(&t).copied().unwrap_or(tier);
        curve.insert(t, tier);
    }
    let rule = entry.map(|e| e.power.clone()).unwrap_or(PowerRule::Auto);
    let mut reserve = 0;
    let mut power = None;
    if let Some(p) = powers.iter().find(|p| p.cost > 0 && !p.exhausted) {
        let usable = match rule {
            PowerRule::Never => false,
            PowerRule::Turns { from_turn, until_turn } => from_turn <= turn && turn <= until_turn,
            PowerRule::Auto => p.cost <= 2 && turn >= 3,
        };
        if usable {
            reserve = p.cost;
            power = Some(p.name.clone());
        }
    }
    Profile {
        curve,
        custom_curve: entry.is_some_and(|e| !e.curve.is_empty()),
        reserve,
        power,
        level_bias: entry.map(|e| e.level_bias).unwrap_or(0.0),
        minion_cost: entry.and_then(|e| e.minion_cost).unwrap_or(MINION_COST),
        buy_bonus: entry.map(|e| e.buy_bonus.clone()).unwrap_or_default(),
        pair_bonus: entry.map(|e| e.pair_bonus).unwrap_or(0.0),
        note: entry.map(|e| e.note.clone()).unwrap_or_default(),
        hero: hero.map(str::to_string),
    }
}

/// (points, reason) of an economy / card advantage card, or (0, ""). Earlier = more turns to profit:
/// ×1.2 up to turn 4, ×1 on turn 6, then less, ×0.3 at the lowest.
pub fn economy_value(k: &Knowledge, card_id: &str, turn: i64) -> (f64, String) {
    let Some(info) = k.economy.get(card_id) else { return (0.0, String::new()) };
    let factor = ((14 - turn) as f64 / 8.0).clamp(0.3, 1.2);
    let word = if info.kind == "gold" { "economy" } else { "cards" };
    (info.value * factor, format!("{word}: {}", info.note).trim().to_string())
}

// ------------------------------------------------------------------- synergy with my board now
pub const RACE_WORDS: [(&str, &str); 11] = [
    ("BEAST", "Beast"),
    ("MECHANICAL", "Mech"),
    ("MURLOC", "Murloc"),
    ("DEMON", "Demon"),
    ("DRAGON", "Dragon"),
    ("ELEMENTAL", "Elemental"),
    ("PIRATE", "Pirate"),
    ("QUILBOAR", "Quilboar"),
    ("UNDEAD", "Undead"),
    ("NAGA", "Naga"),
    ("ABERRATION", "Aberration"),
];

pub fn race_word(race: &str) -> Option<&'static str> {
    RACE_WORDS.iter().find(|(r, _)| *r == race).map(|(_, w)| *w)
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid regex")
}

/// Mechanics: (what has / produces it, what profits from it). A card that profits from a mechanic my
/// board has (or the other way round) = synergy.
static MECHANICS: LazyLock<Vec<(&'static str, Regex, Regex)>> = LazyLock::new(|| {
    [
        ("Deathrattle", r"^Deathrattle|\bDeathrattle:", r"(your|friendly|a|another)\b[^.]*\bDeathrattle|Deathrattles"),
        // (Python had a lookahead "(?!:)"; the regex crate has none: "not followed by ':'" instead.)
        ("Battlecry", r"^Battlecry|\bBattlecry:", r"(your|friendly|a)\b[^.]*\bBattlecr(?:y|ies)\b(?:[^:]|$)|Battlecries"),
        ("Divine Shield", r"Divine Shield", r"(loses?|lost) Divine Shield|with Divine Shield"),
        ("Tavern spell", r"(Get|get|Discover)[^.]*(Tavern spell|spell)", r"(cast|buy)[^.]*Tavern spell|Tavern spells give"),
        ("Blood Gem", r"(Get|get|plays?)[^.]*Blood Gem", r"Blood Gems (give|played)|Whenever a Blood Gem"),
        ("Magnetic", r"Magnetic|Volumizer", r"Magnetize|Magnetic Mech"),
        ("Golden", r"Make[^.]*Golden|Golden minion", r"for each Golden|Golden minions? you"),
    ]
    .into_iter()
    .map(|(n, h, c)| (n, re(h), re(c)))
    .collect()
});
const SYNERGY_POINT: f64 = 0.6;
const SYNERGY_MAX: f64 = 2.5;

static GIVE_WORDS: LazyLock<Regex> = LazyLock::new(|| re(r"\b(Get|get|Discover|discover|Summon|summon)\b[^.]*"));
static RACE_RE: LazyLock<Vec<(&'static str, Regex)>> =
    LazyLock::new(|| RACE_WORDS.iter().map(|(r, w)| (*r, re(&format!(r"\b{w}s?\b")))).collect());

/// Does the text profit from this tribe? ("your Beasts", "after you play a Murloc"). Giving or summoning
/// a tribe ("Get a random Beast", "Summon a 1/1 Murloc") is not profiting from it.
pub fn mentions_race(text: &str, race: &str) -> bool {
    let Some((_, word)) = RACE_RE.iter().find(|(r, _)| *r == race) else { return false };
    let text = GIVE_WORDS.replace_all(text, "");
    word.is_match(&text)
}

fn effective_races(card: &Card) -> Vec<&str> {
    let own: Vec<&str> = card.races.iter().map(String::as_str).filter(|r| *r != "ALL").collect();
    if own.is_empty() && card.has_race("ALL") { RACE_WORDS.iter().map(|(r, _)| *r).collect() } else { own }
}

/// (points, reason): how well this shop card goes with my board (and hand) NOW, even if it serves no
/// comp later. E.g. a Beast when I have "your Beasts" cards.
pub fn board_synergy(card: &Card, board: &[Arc<Card>]) -> (f64, String) {
    let text = card.text.as_str();
    let races = effective_races(card);
    let mut points = 0.0;
    let mut why: Vec<String> = Vec::new();
    for b in board {
        if b.id == card.id {
            continue;
        }
        let btext = b.text.as_str();
        let b_races = effective_races(b);
        let hit = if races.iter().any(|r| mentions_race(btext, r)) || b_races.iter().any(|r| mentions_race(text, r)) {
            Some(b.name.clone())
        } else {
            MECHANICS
                .iter()
                .find(|(_, has, cares)| (has.is_match(btext) && cares.is_match(text)) || (has.is_match(text) && cares.is_match(btext)))
                .map(|(name, _, _)| format!("{} ({name})", b.name))
        };
        if let Some(h) = hit {
            points += SYNERGY_POINT;
            why.push(h);
        }
    }
    let points = points.min(SYNERGY_MAX);
    if why.is_empty() {
        return (points, String::new());
    }
    let more = if why.len() > 2 { format!(" +{}", why.len() - 2) } else { String::new() };
    (points, format!("synergy now: {}{more}", why[..why.len().min(2)].join(", ")))
}

// ---------------------------------------------------------------------------- expected board
pub fn board_power(board: &[MinionSnap]) -> i64 {
    board.iter().map(|m| m.attack + m.health).sum()
}

/// Attack + health of a typical board at this turn and tier (median of lobbies).
pub fn typical_power(turn: i64, tier: i64) -> f64 {
    let (a, b, c) = GROWTH;
    (a + b * turn as f64 + c * tier.max(1) as f64).exp()
}

/// Where this board stands among the lobby's boards at the same turn (0-100).
pub fn strength_percentile(power: i64, turn: i64, tier: i64) -> f64 {
    if power <= 0 {
        return 0.0;
    }
    let z = ((power as f64).ln() - typical_power(turn, tier).ln()) / GROWTH_SD;
    50.0 * (1.0 + erf(z / std::f64::consts::SQRT_2))
}

/// A minion without effect (unknown): only stats.
fn filler(attack: i64, health: i64, tier: i64) -> MinionSnap {
    MinionSnap {
        card_id: String::new(),
        attack,
        health,
        max_health: health,
        golden: false,
        tier,
        races: vec![],
        taunt: false,
        ds: false,
        venom: false,
        poison: false,
        reborn: false,
        stealth: false,
        cleave: false,
        windfury: 0,
        data: (0, 0),
    }
}

/// (likely board of the opponent at this turn, explanation).
///
/// `last` = their last fight against me, frozen. Their minions keep their effects; their stats are
/// multiplied to match the strength expected now, and minions without effect fill the board if they had
/// fewer than usual at this turn.
pub fn expected_board(turn: i64, tier: i64, last: Option<&Fight>) -> (Vec<MinionSnap>, String) {
    board_at(turn, tier, last, 0.0)
}

/// The same board at several strengths around the expected one (equally likely), because the real
/// board is often half or twice as strong: one board alone gives 0% or 100% far too often. Simulating
/// all of them and averaging gives odds that match real fights (see BOARD_SD). Their board this turn
/// (seen during the fight) is known: one board.
pub fn expected_boards(turn: i64, tier: i64, last: Option<&Fight>) -> (Vec<Vec<MinionSnap>>, String) {
    let (board, basis) = board_at(turn, tier, last, 0.0);
    let seen = last.and_then(|f| f.board.as_ref()).is_some_and(|b| !b.is_empty());
    if seen && last.is_some_and(|f| f.turn == turn) {
        return (vec![board], basis);
    }
    let sd = if seen && last.is_some_and(|f| f.turn >= 4) { BOARD_SD.1 } else { BOARD_SD.0 };
    let boards = SPREAD.iter().map(|z| if *z == 0.0 { board.clone() } else { board_at(turn, tier, last, z * sd).0 }).collect();
    (boards, basis)
}

/// `shift` is added to the log of the strength aimed for (+0.7 = twice as strong).
fn board_at(turn: i64, tier: i64, last: Option<&Fight>, shift: f64) -> (Vec<MinionSnap>, String) {
    let tier = tier.max(1);
    let mut target = typical_power(turn, tier).ln() + shift;
    let seen: Vec<MinionSnap> = last.and_then(|f| f.board.clone()).unwrap_or_default();
    let last_turn = last.map(|f| f.turn).unwrap_or(0);
    if !seen.is_empty() && last_turn >= 4 {
        let opp_tier = last.and_then(|f| f.frozen.as_ref()).map(|fr| fr.opp_tier).unwrap_or(1);
        let then = (board_power(&seen).max(1) as f64).ln() - typical_power(last_turn, opp_tier).ln();
        target += PERSISTENCE.powf((turn - last_turn) as f64 / 2.0) * then + SEEN_BOOST;
    } else {
        target += TYPICAL_BOOST;
    }
    let power = target.exp();
    let fresh = !seen.is_empty() && last_turn == turn; // seen this turn (during a fight): nothing to age
    let count = if fresh { seen.len() } else { seen.len().max(board_count(turn)) };
    let mut board = seen.clone();
    while board.len() < count {
        board.push(filler(1, 1, (tier - 1).max(1)));
    }
    let k = if fresh { 1.0 } else { power / board_power(&board).max(1) as f64 };
    for m in &mut board {
        m.attack = ((m.attack as f64 * k).round_ties_even() as i64).max(0);
        m.health = ((m.health as f64 * k).round_ties_even() as i64).max(1);
        m.max_health = m.health;
    }
    let basis = if seen.is_empty() {
        "typical board for this turn (never fought)".to_string()
    } else if fresh {
        "their board this turn".to_string()
    } else {
        format!("their turn {last_turn} board grown ×{k:.1}")
    };
    (board, basis)
}

// --------------------------------------------------------------------------------- simulations
/// Value for the rest of the game of the stats added to my board (they stay after this fight): 3 points
/// for one more typical board of this turn, at most 2. E.g. a 3/3 on turn 3 ≈ 1 point.
pub fn board_value(gain: i64, turn: i64, tier: i64) -> f64 {
    (3.0 * gain as f64 / typical_power(turn, tier)).clamp(-2.0, 2.0)
}

/// Value of a simulated fight (higher = better): win, hurt, and above all don't lose health when little
/// is left (damage taken weighs 1/health: 5 damage at 30 health ≈ 0.17, at 8 health ≈ 0.6).
pub fn fight_value(sim: &SimResult, health: Option<i64>) -> f64 {
    let h = health.filter(|h| *h != 0).unwrap_or(30).max(3) as f64;
    sim.win / 100.0 + 0.02 * sim.avg_dealt - sim.avg_taken / h - 3.0 * sim.lethal_me / 100.0
}

/// My board with this minion added; full board: it replaces the weakest (if better).
pub fn add_minion(team: &[MinionSnap], m: &MinionSnap) -> Vec<MinionSnap> {
    if team.len() < MAX_BOARD {
        let mut out = team.to_vec();
        out.push(m.clone());
        return out;
    }
    let power = |x: &MinionSnap| x.attack + x.health;
    let weakest = (0..team.len()).min_by_key(|&i| power(&team[i])).expect("non-empty team");
    if power(&team[weakest]) >= power(m) {
        return team.to_vec();
    }
    let mut out: Vec<MinionSnap> = team.iter().enumerate().filter(|(i, _)| *i != weakest).map(|(_, x)| x.clone()).collect();
    out.push(m.clone());
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Buys {
    pub buys: Vec<String>,
    pub sim: SimResult,
}

/// A minion of my board, weakest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Weak {
    pub base_id: String,
    pub name: String,
    pub power: i64,
    pub golden: bool,
}

/// Simulations of the turn, for `turn_plan`.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub turn: i64,
    pub tier: i64,
    pub gold: i64,
    pub health: Option<i64>,
    pub cap: Option<i64>,
    pub upgrade_cost: Option<i64>,
    pub reroll_cost: i64,
    pub opponent: Option<String>,
    pub opp_tier: i64,
    pub opp_health: Option<i64>,
    pub basis: String,
    pub opp_power: i64,
    pub my_power: i64,
    pub strength: f64,
    pub base: SimResult,
    /// Card id -> odds with it added (shop order).
    pub per_card: Vec<(String, SimResult)>,
    pub power_gain: Vec<(String, i64)>,
    pub spend: Buys,
    pub level: Option<Buys>,
    pub team_size: usize,
    pub board_size: usize,
    pub weakest: Vec<Weak>,
    pub profile: Profile,
    pub budget: i64,
    pub team_ids: Vec<String>,
}

/// Results kept while nothing changes (one per game).
#[derive(Default)]
pub struct Caches {
    plan: Option<(String, Arc<Plan>)>,
    choice: Option<(String, Arc<OptionFights>)>,
    preview: Option<(String, Odds)>,
}

#[allow(clippy::too_many_arguments)]
fn sim_input(game: &Game, mine: Vec<MinionSnap>, theirs: Vec<MinionSnap>, my_tier: i64, opp_tier: i64, opp: i64, cap: Option<i64>, counters: [Counters; 2]) -> SimInput {
    let bonuses = game.my_bonuses();
    let deity = |pid: Option<i64>| pid.and_then(|p| game.deity_seen.get(&p).copied()).unwrap_or((1, 1));
    SimInput {
        mine,
        theirs,
        my_tier,
        their_tier: opp_tier,
        my_health: game.my_health(),
        their_health: game.player_health(opp),
        first: None,
        deity: game.deity().and_then(|d| combat::deity_id(&d)).map(str::to_string),
        deity_stats: [deity(game.my_player_id()), deity(Some(opp))],
        counters,
        gem: [bonuses.gem, (1, 1)],
        spell: [bonuses.spell, (0, 0)],
        cap,
    }
}

/// My next opponent's tier and last frozen fight against me.
fn opponent_info(game: &Game, opp: i64) -> (i64, Option<&Fight>) {
    let last = game.last_fight_against(opp).filter(|f| f.frozen.is_some());
    let tier = game
        .hero_of(opp)
        .and_then(|h| h.opt_int("PLAYER_TECH_LEVEL"))
        .filter(|t| *t != 0)
        .or_else(|| last.and_then(|f| f.frozen.as_ref()).map(|fr| fr.opp_tier))
        .unwrap_or(1);
    (tier, last)
}

/// Simulations of the turn (None outside the recruit phase). Kept while nothing changes.
pub fn fight_plan(k: &Knowledge, game: &Game, caches: &mut Caches) -> Option<Arc<Plan>> {
    if !game.is_battlegrounds() || game.is_duos() || game.is_over() || game.in_combat() || !game.hero_options().is_empty() {
        return None;
    }
    let hero = game.my_hero()?;
    let shop = game.shop();
    let opp = game.next_opponent_id()?;
    if shop.is_empty() {
        return None;
    }
    let db = cards::db();
    let (turn, tier, gold, health) = (game.turn(), game.tavern_tier().unwrap_or(1).max(1), game.my_gold().unwrap_or(0), game.my_health());
    let (opp_tier, last) = opponent_info(game, opp);
    let mut team: Vec<MinionSnap> = game.my_board().iter().map(|e| snapshot_minion(e, &db)).collect();
    let room = MAX_BOARD.saturating_sub(team.len());
    team.extend(game.my_hand().iter().filter(|e| e.is_minion()).take(room).map(|e| snapshot_minion(e, &db)));
    let hero_name = hero.name();
    let powers: Vec<Power> = game
        .my_hero_powers()
        .iter()
        .map(|e| Power { name: e.name(), card_id: e.card_id.clone(), cost: e.int("COST"), exhausted: e.truthy("EXHAUSTED") })
        .collect();
    let profile = hero_profile(k, Some(&hero_name), &powers, turn);
    let budget = (gold - profile.reserve).max(0); // the hero power's gold is kept aside
    let offers: Vec<(Arc<Card>, MinionSnap, i64)> = shop
        .iter()
        .filter(|e| !e.card_id.is_empty())
        .filter_map(|e| {
            let card = db.base(&e.card_id)?.clone();
            let cost = e.opt_int("COST").filter(|c| *c != 0).unwrap_or(profile.minion_cost);
            Some((card, snapshot_minion(e, &db), cost))
        })
        .collect();
    let (up, roll) = (game.upgrade_cost(), game.reroll_cost());
    let cap = game.damage_cap();
    let key = format!(
        "{team:?}|{:?}|{gold}|{up:?}|{opp}|{opp_tier}|{turn}|{cap:?}|{health:?}|{:?}|{hero_name}|{powers:?}|{}",
        offers.iter().map(|o| &o.1).collect::<Vec<_>>(),
        last.map(|f| f.turn),
        cards::generation(),
    );
    if let Some((k0, plan)) = &caches.plan {
        if *k0 == key {
            return Some(plan.clone());
        }
    }

    let (theirs, basis) = expected_boards(turn, opp_tier, last);
    let counters = [
        game.player_counters(game.local_player_id()),
        last.and_then(|f| f.frozen.as_ref()).map(|fr| fr.counters.1.clone()).unwrap_or_default(),
    ];
    let sim = |mine: &[MinionSnap]| {
        let inputs: Vec<SimInput> =
            theirs.iter().map(|t| sim_input(game, mine.to_vec(), t.clone(), tier, opp_tier, opp, cap, counters.clone())).collect();
        combat::simulate_mix(&db, &inputs, PLAN_RUNS.0, PLAN_RUNS.1, None)
    };
    let base = sim(&team);
    let mut per_card: Vec<(String, SimResult)> = Vec::new();
    let mut gains: Vec<(String, i64)> = Vec::new();
    for (card, m, _) in &offers {
        if per_card.iter().all(|(id, _)| *id != card.id) {
            let board = add_minion(&team, m);
            per_card.push((card.id.clone(), sim(&board)));
            gains.push((card.id.clone(), board_power(&board) - board_power(&team)));
        }
    }
    let value_of = |id: &str| fight_value(&per_card.iter().find(|(i, _)| i == id).expect("simulated").1, health);
    let base_value = fight_value(&base, health);
    // Greedy buys (best fight value first) with this gold.
    let best_buys = |mut money: i64| -> (Vec<String>, Vec<MinionSnap>) {
        let mut ranked: Vec<&(Arc<Card>, MinionSnap, i64)> = offers.iter().collect();
        ranked.sort_by(|a, b| value_of(&b.0.id).partial_cmp(&value_of(&a.0.id)).unwrap_or(std::cmp::Ordering::Equal));
        let mut chosen = Vec::new();
        let mut board = team.clone();
        for (card, m, cost) in ranked {
            if *cost <= money && value_of(&card.id) > base_value {
                chosen.push(card.id.clone());
                board = add_minion(&board, m);
                money -= cost;
            }
        }
        (chosen, board)
    };
    let (spend_ids, spend_board) = best_buys(budget);
    let spend = match spend_ids.len() {
        0 => base.clone(),
        1 => per_card.iter().find(|(i, _)| *i == spend_ids[0]).expect("simulated").1.clone(),
        _ => sim(&spend_board),
    };
    let level = match up {
        Some(up) if budget >= up => {
            let (ids, board) = best_buys(budget - up);
            let s = if ids.is_empty() { base.clone() } else { sim(&board) };
            Some(Buys { buys: ids, sim: s })
        }
        _ => None,
    };
    let opp_hero = game.hero_of(opp);
    let board_entities = game.my_board();
    let plan = Arc::new(Plan {
        turn,
        tier,
        gold,
        health,
        cap,
        upgrade_cost: up,
        reroll_cost: roll,
        opponent: opp_hero.map(|h| h.name()),
        opp_tier,
        opp_health: game.player_health(opp),
        basis,
        opp_power: board_power(&theirs[theirs.len() / 2]),
        my_power: board_power(&team),
        strength: strength_percentile(board_power(&team), turn, tier),
        base,
        per_card,
        power_gain: gains,
        spend: Buys { buys: spend_ids, sim: spend },
        level,
        team_size: team.len(),
        board_size: board_entities.len(),
        weakest: by_weakness(&board_entities, &db),
        profile,
        budget,
        team_ids: board_entities
            .iter()
            .chain(game.my_hand().iter())
            .filter(|e| !e.card_id.is_empty() && e.is_minion())
            .filter_map(|e| db.base(&e.card_id).map(|c| c.id.clone()))
            .collect(),
    });
    caches.plan = Some((key, plan.clone()));
    Some(plan)
}

/// Next fight with each option of a choice (Discover) added to my board, against the same expected board
/// as the turn plan.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionFights {
    pub turn: i64,
    pub tier: i64,
    pub health: Option<i64>,
    pub base: SimResult,
    /// Per option (same order): odds with it added, and attack + health it adds to my team.
    pub with: Vec<(SimResult, i64)>,
    pub team: Vec<String>,
}

/// Simulates my next fight with each minion option added (None in combat, or without a next opponent).
/// `options` = the option entities; non-minions get the base odds and no gain. Kept while nothing changes.
pub fn option_fights(game: &Game, options: &[&crate::game::Entity], caches: &mut Caches) -> Option<Arc<OptionFights>> {
    if !game.is_battlegrounds() || game.is_duos() || game.is_over() || game.in_combat() {
        return None;
    }
    let opp = game.next_opponent_id()?;
    let db = cards::db();
    let (turn, tier, health) = (game.turn(), game.tavern_tier().unwrap_or(1).max(1), game.my_health());
    let (opp_tier, last) = opponent_info(game, opp);
    let mut team: Vec<MinionSnap> = game.my_board().iter().map(|e| snapshot_minion(e, &db)).collect();
    let room = MAX_BOARD.saturating_sub(team.len());
    team.extend(game.my_hand().iter().filter(|e| e.is_minion()).take(room).map(|e| snapshot_minion(e, &db)));
    let snaps: Vec<Option<MinionSnap>> = options.iter().map(|e| e.is_minion().then(|| snapshot_minion(e, &db))).collect();
    let cap = game.damage_cap();
    let key = format!("{team:?}|{snaps:?}|{opp}|{opp_tier}|{turn}|{cap:?}|{health:?}|{:?}|{}", last.map(|f| f.turn), cards::generation());
    if let Some((k0, f)) = &caches.choice {
        if *k0 == key {
            return Some(f.clone());
        }
    }
    let (theirs, _) = expected_boards(turn, opp_tier, last);
    let counters = [
        game.player_counters(game.local_player_id()),
        last.and_then(|f| f.frozen.as_ref()).map(|fr| fr.counters.1.clone()).unwrap_or_default(),
    ];
    let sim = |mine: &[MinionSnap]| {
        let inputs: Vec<SimInput> =
            theirs.iter().map(|t| sim_input(game, mine.to_vec(), t.clone(), tier, opp_tier, opp, cap, counters.clone())).collect();
        combat::simulate_mix(&db, &inputs, PLAN_RUNS.0, PLAN_RUNS.1, None)
    };
    let base = sim(&team);
    let with = snaps
        .iter()
        .map(|m| match m {
            Some(m) => {
                let board = add_minion(&team, m);
                (sim(&board), board_power(&board) - board_power(&team))
            }
            None => (base.clone(), 0),
        })
        .collect();
    let f = Arc::new(OptionFights { turn, tier, health, base, with, team: team.iter().map(|m| db.base_id(&m.card_id).to_string()).collect() });
    caches.choice = Some((key, f.clone()));
    Some(f)
}

/// My board, weakest first; golden ones last.
fn by_weakness(board: &[&crate::game::Entity], db: &CardDb) -> Vec<Weak> {
    let power = |e: &crate::game::Entity| e.int("ATK") + e.int("HEALTH") - e.int("DAMAGE");
    let mut ranked: Vec<&&crate::game::Entity> = board.iter().collect();
    ranked.sort_by_key(|e| (e.is_golden(), power(e)));
    ranked
        .into_iter()
        .map(|e| Weak {
            base_id: db.base_id(&e.card_id).to_string(),
            name: e.name(),
            power: power(e),
            golden: e.is_golden(),
        })
        .collect()
}

/// (name, reason) of the minion to sell to make room, or None.
///
/// Kept: cards of my comp and pairs (protect), golden ones, economy cards that pay while on board.
/// Sold first: cards that pay when sold, those whose economy Battlecry is already played, then the
/// weakest.
pub fn sell_choice(k: &Knowledge, board: &[Weak], protect: &HashSet<String>, turn: i64, tier: i64) -> Option<(String, String)> {
    let mut best: Option<(f64, String, String)> = None;
    for w in board {
        if protect.contains(&w.base_id) || w.golden {
            continue;
        }
        let info = k.economy.get(&w.base_id);
        let mut keep = board_value(w.power, turn, tier);
        let mut why = "weakest".to_string();
        match info.map(|i| i.when.as_str()) {
            Some("board") => keep += economy_value(k, &w.base_id, turn).0,
            Some("sell") => {
                keep -= 1.5;
                why = info.map(|i| i.note.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "reward when sold".into());
            }
            Some("play") => {
                keep -= 0.5;
                why = "its Battlecry is already used".into();
            }
            _ => {}
        }
        if best.as_ref().is_none_or(|b| keep < b.0) {
            best = Some((keep, w.name.clone(), why));
        }
    }
    best.map(|(_, n, w)| (n, w))
}

// ------------------------------------------------------------------------------- roll chances
/// Chance to see at least one of the wanted cards in the next shop. `available`: every pool card of this
/// lobby up to my tier. Each shop slot draws a random minion among the copies left (pool assumed full).
pub fn find_chance(wanted: &[Arc<Card>], tier: i64, available: &[Arc<Card>]) -> f64 {
    let total: f64 = available.iter().map(|c| pool_copies(c.tier)).sum();
    let ids: HashSet<&str> = available.iter().map(|c| c.id.as_str()).collect();
    let mut unique: BTreeMap<&str, &Arc<Card>> = BTreeMap::new();
    for c in wanted {
        unique.insert(c.id.as_str(), c);
    }
    let hit: f64 = unique.values().filter(|c| ids.contains(c.id.as_str())).map(|c| pool_copies(c.tier)).sum();
    if total == 0.0 || hit == 0.0 {
        return 0.0;
    }
    1.0 - (1.0 - hit / total).powi(shop_slots(tier))
}

// ----------------------------------------------------------------------------------- advice
#[derive(Debug, Clone, PartialEq)]
pub struct Danger {
    pub level: &'static str,
    pub color: &'static str,
    pub text: String,
}

/// How much the next fight threatens my health.
pub fn danger_level(plan: &Plan) -> Danger {
    let s = &plan.base;
    let hp = plan.health.unwrap_or(0);
    let d = |level, color, text| Danger { level, color, text };
    if s.lethal_me >= 10.0 {
        return d("critical", "#e57373", format!("{:.0}% chance the next fight kills you", s.lethal_me));
    }
    if hp != 0 && s.loss >= 50.0 && s.taken >= hp as f64 / 3.0 {
        return d("high", "#e0a050", format!("likely loss for ~{:.0} of your {hp} health", s.taken));
    }
    if s.loss >= 50.0 && s.taken >= hp as f64 / 6.0 {
        return d("medium", "#d9c060", format!("likely loss (~{:.0} damage)", s.taken));
    }
    if s.loss >= 50.0 {
        return d("low", "#7fe07f", format!("likely loss, but only ~{:.0} damage", s.taken));
    }
    d("low", "#7fe07f", "you should win or trade".into())
}

/// (level?, reason), or (None, reason) when leveling is not possible this turn.
pub fn level_advice(plan: &Plan, foregone: f64) -> (Option<bool>, String) {
    let (tier, turn) = (plan.tier, plan.turn);
    let profile = &plan.profile;
    let gold = plan.budget;
    let mc = profile.minion_cost;
    let Some(up) = plan.upgrade_cost else { return (None, "max tier".into()) };
    let Some(level) = &plan.level else {
        let after = if profile.reserve > 0 { " after your hero power" } else { "" };
        return (None, format!("level costs {up}, you have {gold}{after}"));
    };
    let hp = plan.health.filter(|h| *h != 0).unwrap_or(30).max(3);
    let (lvl, spend) = (&level.sim, &plan.spend.sim);
    let curve = profile.curve.get(&turn).copied().unwrap_or(6);
    let behind = curve - (tier + 1); // >= 0: leveling keeps me on the curve
    let mut benefit = match behind {
        0 => 2.0,
        b if b > 0 => 3.0,
        _ => 0.7,
    } + profile.level_bias;
    if up <= 2 {
        benefit += 1.0; // almost free
    }
    if gold - up >= mc {
        benefit += 0.5; // I can still buy
    }
    let extra = (lvl.avg_taken - spend.avg_taken).max(0.0);
    let risk = extra * 10.0 / (hp - 5).max(1) as f64 + (lvl.lethal_me - spend.lethal_me).max(0.0) / 5.0 + foregone;
    if lvl.lethal_me >= 15.0 && lvl.lethal_me > spend.lethal_me + 5.0 {
        return (
            Some(false),
            format!("leveling leaves you {:.0}% to die (buying: {:.0}%): buy instead", lvl.lethal_me, spend.lethal_me),
        );
    }
    if hp <= 15 && tier >= 4 && behind <= 0 && plan.base.loss >= 50.0 {
        return (Some(false), format!("only {} health and you're losing: spend on your board now", plan.health.unwrap_or(0)));
    }
    let mut place = match behind {
        0 => "on curve".to_string(),
        b if b > 0 => format!("{b} tier behind curve"),
        _ => "ahead of curve".to_string(),
    };
    if profile.custom_curve {
        place = place.replace("curve", &format!("{}'s curve", profile.hero.as_deref().unwrap_or("hero")));
    }
    let mut costs = Vec::new();
    if extra >= 0.5 {
        costs.push(format!("costs ~{extra:.1} health this fight"));
    }
    if foregone >= 0.3 {
        costs.push("skips good buys".to_string());
    }
    if benefit > risk {
        let tail = if costs.is_empty() { String::new() } else { format!(" ({})", costs.join(", ")) };
        return (Some(true), format!("Tier {} for {up} gold: {place}{tail}", tier + 1));
    }
    let what = if costs.is_empty() { "isn't worth it yet".to_string() } else { costs.join(" and ") };
    (Some(false), format!("wait ({place}): leveling {what}"))
}

/// One buy suggestion.
#[derive(Debug, Clone, PartialEq)]
pub struct Buy {
    pub card: Arc<Card>,
    /// Win chance gained, in points of %.
    pub gain: f64,
    pub value: f64,
    pub reasons: Vec<String>,
    /// What stays after this fight (comp, stats, economy, synergy).
    pub lasting: f64,
}

/// Readable advice for the turn.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnAdvice {
    pub level: Option<bool>,
    pub level_why: String,
    /// Best buys shown (3 at most).
    pub buy: Vec<Buy>,
    pub plan_buys: Vec<Arc<Card>>,
    pub sell: Option<String>,
    pub roll: Option<String>,
    pub roll_pro: String,
    pub freeze: Option<String>,
    pub danger: Danger,
    pub power: Option<String>,
    pub hero_note: String,
}

/// Readable advice of the turn.
///
/// comp_picks: advice::shop_advice (comp / triple points of each card); wanted: cards I look for
/// (missing core cards of my comp, 3rd copy of a pair); available: pool cards of this lobby up to my
/// tier (for roll chances); protect: ids of cards not to sell (my comp's cards, pairs).
pub fn turn_plan(
    k: &Knowledge,
    plan: &Plan,
    comp_picks: &[crate::advice::ShopPick],
    wanted: &[Arc<Card>],
    available: &[Arc<Card>],
    protect: &HashSet<String>,
) -> TurnAdvice {
    let db = cards::db();
    let hp = plan.health;
    let profile = &plan.profile;
    let gold = plan.budget; // without the gold kept for the hero power
    let mc = profile.minion_cost.max(1);
    let base = &plan.base;
    let team: Vec<Arc<Card>> = plan.team_ids.iter().filter_map(|i| db.get(i).cloned()).collect();
    // Value of each card: comp points (0-6) + stats + economy + gain in the next fight (+20% win = 1 point)
    let mut buys: Vec<Buy> = Vec::new();
    for (cid, s) in &plan.per_card {
        let Some(card) = db.get(cid).cloned() else { continue };
        let gain = s.win - base.win;
        let saved = base.avg_taken - s.avg_taken;
        let pick = comp_picks.iter().find(|p| p.card.id == *cid);
        let power_gain = plan.power_gain.iter().find(|(i, _)| i == cid).map(|(_, g)| *g).unwrap_or(0);
        let stats = board_value(power_gain, plan.turn, plan.tier);
        let (econ, econ_why) = economy_value(k, cid, plan.turn);
        let (syn, syn_why) = board_synergy(&card, &team);
        let mut lasting = pick.map(|p| p.value).unwrap_or(0.0) + stats + econ + syn;
        let mut value = lasting + FIGHT_WEIGHT * (fight_value(s, hp) - fight_value(base, hp));
        let mut reasons: Vec<String> = pick.map(|p| p.reasons.clone()).unwrap_or_default();
        let mut hero_bonus = profile
            .buy_bonus
            .iter()
            .filter(|(race, _)| card.races.contains(race))
            .map(|(_, v)| *v)
            .fold(0.0, f64::max);
        if profile.pair_bonus != 0.0 && reasons.iter().any(|r| r == "pair") {
            hero_bonus += profile.pair_bonus;
        }
        if hero_bonus != 0.0 {
            lasting += hero_bonus;
            value += hero_bonus;
            let note = profile.note.split(':').next().unwrap_or("");
            reasons.push(format!("{}: {note}", profile.hero.as_deref().unwrap_or("hero")));
        }
        if !econ_why.is_empty() {
            reasons.push(econ_why);
        }
        if !syn_why.is_empty() {
            reasons.push(syn_why);
        }
        if stats >= 0.5 && gain < 1.0 {
            reasons.push("stats for later fights".into());
        }
        let mut fight = Vec::new();
        if gain >= 1.0 {
            fight.push(format!("+{gain:.0}% win"));
        }
        if saved >= 0.5 {
            fight.push(format!("-{saved:.1} dmg taken"));
        }
        if !fight.is_empty() {
            reasons.push(fight.join(", "));
        }
        if value > 0.3 {
            buys.push(Buy { card, gain, value, reasons, lasting });
        }
    }
    buys.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

    // What leveling makes me miss: the value of the buys the gold left no longer pays for.
    let mut values: Vec<f64> = buys.iter().map(|b| b.lasting).collect();
    values.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let up = plan.upgrade_cost.unwrap_or(0);
    let sum_first = |n: i64| values.iter().take(n.max(0) as usize).sum::<f64>();
    let foregone = 0.6 * (sum_first(gold.div_euclid(mc)) - sum_first((gold - up).max(0).div_euclid(mc)));
    let (want_level, level_why) = level_advice(plan, foregone);
    let mut money = gold - if want_level == Some(true) { plan.upgrade_cost.unwrap_or(0) } else { 0 };
    let affordable = (money.max(0) / mc) as usize;
    let shown: Vec<Buy> = buys.iter().take(affordable.max(1) + 1).cloned().collect();
    let plan_buys: Vec<Buy> = buys.iter().take(affordable).cloned().collect();
    let mut sell = None;
    if !plan_buys.is_empty() && plan.team_size + plan_buys.len() > MAX_BOARD {
        if let Some((name, why)) = sell_choice(k, &plan.weakest, protect, plan.turn, plan.tier) {
            sell = Some(format!("{name} ({why})"));
            money += 1; // selling gives 1
        }
    }
    let left = money - mc * plan_buys.len() as i64;
    // Roll: gold not spent is lost at the end of the turn
    let roll_cost = plan.reroll_cost;
    let chance = if wanted.is_empty() { 0.0 } else { find_chance(wanted, plan.tier, available) };
    let roll = if roll_cost == 0 {
        Some("Free roll: always use it".to_string())
    } else if !plan_buys.is_empty() && left >= roll_cost {
        Some(format!("Then roll {}× with your last {left} gold (unspent gold is lost)", left / roll_cost))
    } else if plan_buys.is_empty() && money >= roll_cost {
        // Nothing worth it here: roll, keeping 3 gold per minion to buy if I find one
        let n = (money - mc).max(0) / roll_cost;
        Some(if n > 0 {
            format!("Nothing here is worth it: roll up to {n}× and keep {mc} gold to buy what you find")
        } else {
            format!("Nothing here is worth it: roll {}×", money / roll_cost)
        })
    } else {
        None
    };
    let roll_pro = if chance > 0.0 { format!("{:.0}% per roll to see a card you want", chance * 100.0) } else { String::new() };
    let keep: Vec<&Buy> = buys.iter().skip(affordable).filter(|b| b.value >= 3.0).collect();
    let freeze = if !keep.is_empty() && left < mc { Some(format!("Freeze to keep {} for next turn", keep[0].card.name)) } else { None };
    let power = profile
        .power
        .as_ref()
        .filter(|_| profile.reserve > 0)
        .map(|p| format!("Use {p} ({} gold, kept aside above)", profile.reserve));
    TurnAdvice {
        level: want_level,
        level_why,
        buy: shown.into_iter().take(3).collect(),
        plan_buys: plan_buys.into_iter().map(|b| b.card).collect(),
        sell,
        roll,
        roll_pro,
        freeze,
        danger: danger_level(plan),
        power,
        hero_note: profile.note.clone(),
    }
}

// ------------------------------------------------------------------------- odds for the HUD
/// Odds of the fight going on, or an estimate before it.
#[derive(Debug, Clone, PartialEq)]
pub struct Odds {
    pub sim: SimResult,
    /// True during the fight (both boards known).
    pub live: bool,
    pub turn: i64,
    pub opponent: Option<String>,
    /// How the opponent's board was guessed (estimates only).
    pub basis: Option<String>,
    /// Turn their board was last seen (estimates only).
    pub seen_turn: Option<i64>,
    pub cap: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FightLine {
    pub turn: i64,
    pub win: f64,
    pub tie: f64,
    pub loss: f64,
    pub result: char,
}

/// What the panel shows: the fight going on (or an estimate before it) and my average odds over the
/// game's fights.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CombatInfo {
    pub fights: Vec<FightLine>,
    /// Average win / tie / loss over the game's fights.
    pub avg: Option<(f64, f64, f64)>,
    pub expected_wins: f64,
    pub actual_wins: usize,
    pub now: Option<Odds>,
}

/// Runs in the log reader thread, never in the window.
pub fn combat_info(game: &mut Game, caches: &mut Caches) -> Option<CombatInfo> {
    if !game.is_battlegrounds() || game.is_duos() {
        return None;
    }
    for i in 0..game.fights.len() {
        combat::fight_odds(game, i, combat::PAST); // past fights: fast, for the game's average
    }
    let done: Vec<FightLine> = game
        .fights
        .iter()
        .filter_map(|f| f.sim.as_ref().map(|s| FightLine { turn: f.turn, win: s.win, tie: s.tie, loss: s.loss, result: f.result }))
        .collect();
    let mut info = CombatInfo::default();
    if !done.is_empty() {
        let n = done.len() as f64;
        let avg = |f: fn(&FightLine) -> f64| done.iter().map(f).sum::<f64>() / n;
        info.avg = Some((avg(|f| f.win), avg(|f| f.tie), avg(|f| f.loss)));
        info.expected_wins = done.iter().map(|f| f.win).sum::<f64>() / 100.0;
        info.actual_wins = done.iter().filter(|f| f.result == 'W').count();
    }
    info.fights = done;

    if let Some(cur) = game.current_fight() {
        let (turn, opponent) = (cur.turn, cur.opponent);
        if cur.sim.is_none() {
            if let Some(input) = combat::fight_input(game, cur) {
                let r = combat::simulate(&input, combat::LIVE.0, combat::LIVE.1, None);
                if let Some(c) = game.current_fight_mut() {
                    c.sim = Some(r);
                }
            }
        }
        let sim = game.current_fight().and_then(|c| c.sim.clone());
        if let Some(sim) = sim {
            info.now = Some(Odds {
                sim,
                live: true,
                turn,
                opponent: opponent.and_then(|o| game.hero_of(o)).map(|h| h.name()),
                basis: None,
                seen_turn: None,
                cap: None,
            });
        }
        return Some(info);
    }
    if game.is_over() {
        return Some(info);
    }
    // Recruit phase: my board against the EXPECTED board of my next opponent.
    let Some(opp) = game.next_opponent_id() else { return Some(info) };
    let db = cards::db();
    let mine: Vec<MinionSnap> = game.my_board().iter().map(|e| snapshot_minion(e, &db)).collect();
    if mine.is_empty() {
        return Some(info);
    }
    let (opp_tier, last) = opponent_info(game, opp);
    let cap = game.damage_cap();
    let key = format!("{mine:?}|{:?}|{opp}|{opp_tier}|{}|{cap:?}|{}", last.map(|f| f.turn), game.turn(), cards::generation());
    let cached = caches.preview.as_ref().filter(|(k0, _)| *k0 == key).map(|(_, o)| o.clone());
    let odds = match cached {
        Some(o) => o,
        None => {
            let (theirs, basis) = expected_boards(game.turn(), opp_tier, last);
            let counters = [
                game.player_counters(game.local_player_id()),
                last.and_then(|f| f.frozen.as_ref()).map(|fr| fr.counters.1.clone()).unwrap_or_default(),
            ];
            let tier = game.tavern_tier().unwrap_or(1).max(1);
            let inputs: Vec<SimInput> =
                theirs.into_iter().map(|t| sim_input(game, mine.clone(), t, tier, opp_tier, opp, cap, counters.clone())).collect();
            let sim = combat::simulate_mix(&db, &inputs, combat::PREVIEW.0, combat::PREVIEW.1, None);
            let o = Odds {
                sim,
                live: false,
                turn: game.turn(),
                opponent: game.hero_of(opp).map(|h| h.name()),
                basis: Some(basis),
                seen_turn: last.map(|f| f.turn),
                cap,
            };
            caches.preview = Some((key, o.clone()));
            o
        }
    };
    info.now = Some(odds);
    Some(info)
}

#[cfg(test)]
mod tests;
