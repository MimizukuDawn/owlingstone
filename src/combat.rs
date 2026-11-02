//! Battlegrounds combat simulator: my odds of winning a fight (like BobsBuddy in HDT).
//!
//! Who does what:
//! - game.rs takes a picture of both boards at the first hit of a fight (ATTACK block in the log). At
//!   that point Start of Combat effects have happened and their buffs are in the stats we read. Each
//!   minion is a `MinionSnap` (card, attack, health, keywords, tier...).
//! - `simulate` replays that fight thousands of times with the game's randomness (targets, random
//!   effects, who starts when both boards have as many minions) and counts wins / ties / losses, the
//!   average damage and the chances of dying (lethal).
//! - Rules: the side with more minions attacks first; sides take turns attacking with their next minion
//!   (left to right); the target is random among Taunts; damage = winner's tavern tier + tiers of the
//!   minions left. Keywords: Taunt, Divine Shield, Windfury (and Mega-), Venomous, Poisonous, Reborn,
//!   Stealth, damage to neighbours.
//! - Known card effects are in the tables at the end (Deathrattles, Rally, Avenge, "when a friendly
//!   minion attacks"...). Board cards whose effect matters in combat but is not coded are listed in
//!   `SimResult::unknown`: the odds are then less reliable (like BobsBuddy's warning).
//!
//! Written from the game's rules, without code from HDT or BobsBuddy.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cards::{self, Card, CardDb};
use crate::game::{Counters, Entity};
use crate::rng::Rng;

const MAX_BOARD: usize = 7;
/// Safety: a stuck fight (only 0-attack minions left) ends in a tie.
const MAX_ATTACKS: u32 = 400;
const BEETLE: &str = "BG28_603t";
/// Fights are split into chunks run in parallel; each chunk has its own random stream.
const CHUNK: u32 = 250;

/// Display settings: the fight going on (like BobsBuddy: 10 000 in 1.5 s at most), past fights of the
/// game (average of the game), estimate during the recruit phase.
pub const LIVE: (u32, f64) = (6000, 1.5);
pub const PAST: (u32, f64) = (1500, 0.3);
pub const PREVIEW: (u32, f64) = (1500, 0.4);

/// A minion as the simulator needs it (taken from the log, or made up by the planner).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MinionSnap {
    pub card_id: String,
    pub attack: i64,
    pub health: i64,
    pub max_health: i64,
    pub golden: bool,
    pub tier: i64,
    pub races: Vec<String>,
    #[serde(default)]
    pub taunt: bool,
    #[serde(default)]
    pub ds: bool,
    #[serde(default)]
    pub venom: bool,
    #[serde(default)]
    pub poison: bool,
    #[serde(default)]
    pub reborn: bool,
    #[serde(default)]
    pub stealth: bool,
    #[serde(default)]
    pub cleave: bool,
    /// 0 = none, 1 = Windfury, 3 = Mega-Windfury.
    #[serde(default)]
    pub windfury: u8,
    /// TAG_SCRIPT_DATA_NUM_1 / _2 (counters some cards show, e.g. Beetle stats).
    #[serde(default)]
    pub data: (i64, i64),
}

impl MinionSnap {
    /// A minion with the card's own stats and keywords.
    pub fn of_card(card: &Card) -> MinionSnap {
        MinionSnap {
            card_id: card.id.clone(),
            attack: card.attack,
            health: card.health,
            max_health: card.health,
            golden: card.id.ends_with("_G"),
            tier: card.tier.filter(|t| *t != 0).unwrap_or(1),
            races: card.races.clone(),
            taunt: card.has_keyword("TAUNT"),
            ds: card.has_keyword("DIVINE_SHIELD"),
            venom: card.has_keyword("VENOMOUS"),
            poison: card.has_keyword("POISONOUS"),
            reborn: card.has_keyword("REBORN"),
            stealth: false,
            cleave: false,
            windfury: if card.has_keyword("WINDFURY") { 1 } else { 0 },
            data: (0, 0),
        }
    }
}

/// Picture of a minion of the log.
pub fn snapshot_minion(e: &Entity, db: &CardDb) -> MinionSnap {
    let card = if e.card_id.is_empty() { None } else { db.base(&e.card_id) };
    let health = e.int("HEALTH");
    let races = match card {
        Some(c) => c.races.clone(),
        None => e.text("CARDRACE").map(|r| vec![r.to_string()]).unwrap_or_default(),
    };
    let tier = e
        .opt_int("TECH_LEVEL")
        .filter(|t| *t != 0)
        .or_else(|| card.map(|c| c.tier.unwrap_or(0)))
        .filter(|t| *t != 0)
        .unwrap_or(1);
    let windfury = if e.truthy("MEGA_WINDFURY") || e.int("WINDFURY") >= 3 {
        3
    } else if e.truthy("WINDFURY") {
        1
    } else {
        0
    };
    MinionSnap {
        card_id: e.card_id.clone(),
        attack: e.int("ATK"),
        health: health - e.int("DAMAGE"),
        max_health: health,
        golden: e.is_golden() || e.card_id.ends_with("_G"),
        tier,
        races,
        taunt: e.truthy("TAUNT"),
        ds: e.truthy("DIVINE_SHIELD"),
        venom: e.truthy("VENOMOUS"),
        poison: e.truthy("POISONOUS"),
        reborn: e.truthy("REBORN"),
        stealth: e.truthy("STEALTH"),
        cleave: e.truthy("CLEAVE"),
        windfury,
        data: (e.int("TAG_SCRIPT_DATA_NUM_1"), e.int("TAG_SCRIPT_DATA_NUM_2")),
    }
}

// ------------------------------------------------------------------------------------------ tribes
const RACES: [&str; 12] = [
    "BEAST", "DEMON", "DRAGON", "ELEMENTAL", "MECHANICAL", "MURLOC", "NAGA", "PIRATE", "QUILBOAR", "UNDEAD", "ABERRATION",
    "TOTEM",
];
const ALL: u16 = 1 << 12;

fn race_bit(race: &str) -> u16 {
    if race == "ALL" {
        return ALL;
    }
    RACES.iter().position(|r| *r == race).map(|i| 1 << i).unwrap_or(0)
}

fn race_mask<S: AsRef<str>>(races: &[S]) -> u16 {
    races.iter().fold(0, |m, r| m | race_bit(r.as_ref()))
}

// ------------------------------------------------------------------------------------------ minions
#[derive(Debug, Clone)]
struct M {
    /// Normal card id (effects are looked up with it).
    base: String,
    card_id: String,
    atk: i64,
    hp: i64,
    max_hp: i64,
    golden: bool,
    tier: i64,
    races: u16,
    taunt: bool,
    ds: bool,
    venom: bool,
    poison: bool,
    reborn: bool,
    stealth: bool,
    cleave: bool,
    windfury: u8,
    side: usize,
    dead: bool,
    /// Who killed it (for Leeroy).
    killer: Option<usize>,
    /// Already triggered once (Scarlet Survivor...).
    flag: bool,
}

impl M {
    fn new(d: &MinionSnap, side: usize, db: &CardDb) -> M {
        let base = if d.card_id.is_empty() {
            String::new()
        } else {
            db.base(&d.card_id).map(|c| c.id.clone()).unwrap_or_else(|| d.card_id.clone())
        };
        M {
            base,
            card_id: d.card_id.clone(),
            atk: d.attack,
            hp: d.health,
            max_hp: d.max_health.max(d.health),
            golden: d.golden,
            tier: if d.tier == 0 { 1 } else { d.tier },
            races: race_mask(&d.races),
            taunt: d.taunt,
            ds: d.ds,
            venom: d.venom,
            poison: d.poison,
            reborn: d.reborn,
            stealth: d.stealth,
            cleave: d.cleave,
            windfury: d.windfury,
            side,
            dead: false,
            killer: None,
            flag: false,
        }
    }

    fn has_race(&self, race: &str) -> bool {
        self.races & (race_bit(race) | ALL) != 0
    }

    /// How many times its effect works: 2 when golden.
    fn g(&self) -> i64 {
        if self.golden { 2 } else { 1 }
    }

    fn stats(mut self, atk: i64, hp: i64) -> M {
        self.atk = atk;
        self.hp = hp;
        self.max_hp = hp;
        self
    }
}

/// Game-long context of one fight, the same for every run.
struct Ctx<'a> {
    db: &'a CardDb,
    beetle: [(i64, i64); 2],
    lobster: [(i64, i64); 2],
    deathrattle_pool: Vec<String>,
    deity_id: Option<String>,
    deity_stats: [(i64, i64); 2],
    knights: [i64; 2],
    undead: [i64; 2],
    gem: [(i64, i64); 2],
    spell: [(i64, i64); 2],
}

// --------------------------------------------------------------------------------------- one fight
/// One fight, played once. sides[0] = me, sides[1] = the opponent. Minions live in `ms`; boards hold
/// their indexes, so a minion keeps its identity when it moves or dies.
struct Fight<'a, 'r> {
    ctx: &'a Ctx<'a>,
    rng: &'r mut Rng,
    ms: Vec<M>,
    sides: [Vec<usize>; 2],
    /// Next attacker of each side (index in the board).
    ptr: [usize; 2],
    deaths: [i64; 2],
    /// minion -> deaths counted since its last Avenge.
    avenge: HashMap<usize, i64>,
    /// Deity: wakes up after 4 friendly Aberrations died.
    aberration_deaths: [i64; 2],
    /// Y'Shaarj: the first dead Aberrations.
    dead_aberrations: [Vec<usize>; 2],
    deity_done: [bool; 2],
    /// Eternal Knights dead in this fight.
    knight_bonus: [i64; 2],
    /// Stalwart Kodo: 3 times per fight.
    kodo_left: HashMap<usize, i64>,
    beetle: [(i64, i64); 2],
    /// "Your Undead have +X Attack this game".
    undead: [i64; 2],
    gem: [(i64, i64); 2],
    /// "Give your Deity +2/+1" before it wakes up.
    deity_bonus: [(i64, i64); 2],
    lobster: [(i64, i64); 2],
}

impl<'a, 'r> Fight<'a, 'r> {
    fn new(protos: &[Vec<M>; 2], ctx: &'a Ctx<'a>, rng: &'r mut Rng) -> Fight<'a, 'r> {
        let mut ms = Vec::with_capacity(protos[0].len() + protos[1].len() + 16);
        let mut sides = [Vec::new(), Vec::new()];
        for side in 0..2 {
            for m in &protos[side] {
                sides[side].push(ms.len());
                ms.push(m.clone());
            }
        }
        Fight {
            ctx,
            rng,
            ms,
            sides,
            ptr: [0, 0],
            deaths: [0, 0],
            avenge: HashMap::new(),
            aberration_deaths: [0, 0],
            dead_aberrations: [Vec::new(), Vec::new()],
            deity_done: [false, false],
            knight_bonus: [0, 0],
            kodo_left: HashMap::new(),
            beetle: ctx.beetle,
            undead: ctx.undead,
            gem: ctx.gem,
            deity_bonus: [(0, 0), (0, 0)],
            lobster: ctx.lobster,
        }
    }

    // -------------------------------------------------------------- tools
    /// A minion summoned in combat, with the card's stats.
    fn token(&self, card_id: &str, side: usize) -> M {
        let card = if card_id.is_empty() { None } else { self.ctx.db.get(card_id) };
        let d = match card {
            Some(c) => MinionSnap::of_card(c),
            None => MinionSnap {
                card_id: card_id.to_string(),
                attack: 1,
                health: 1,
                max_health: 1,
                golden: card_id.ends_with("_G"),
                tier: 1,
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
            },
        };
        M::new(&d, side, self.ctx.db)
    }

    fn alive(&self, side: usize) -> Vec<usize> {
        self.sides[side].iter().copied().filter(|&i| !self.ms[i].dead).collect()
    }

    fn any_alive(&self, side: usize) -> bool {
        self.sides[side].iter().any(|&i| !self.ms[i].dead)
    }

    fn pos(&self, i: usize) -> Option<usize> {
        self.sides[self.ms[i].side].iter().position(|&x| x == i)
    }

    fn summon(&mut self, side: usize, mut m: M, pos: Option<usize>, aura: bool) -> Option<usize> {
        if self.sides[side].len() >= MAX_BOARD {
            return None;
        }
        if aura && m.has_race("UNDEAD") {
            m.atk += self.undead[side]; // the "this game" bonus also counts for new ones
        }
        m.side = side;
        let len = self.sides[side].len();
        let pos = pos.map(|p| p.min(len)).unwrap_or(len);
        let i = self.ms.len();
        self.ms.push(m);
        self.sides[side].insert(pos, i);
        if pos < self.ptr[side] {
            self.ptr[side] += 1;
        }
        self.on_summon(side, i);
        Some(i)
    }

    fn remove(&mut self, i: usize) -> usize {
        let side = self.ms[i].side;
        let pos = self.pos(i).expect("minion on its board");
        self.sides[side].remove(pos);
        if pos < self.ptr[side] {
            self.ptr[side] -= 1;
        }
        pos
    }

    fn pick(&mut self, side: usize, exclude: Option<usize>, cond: impl Fn(&M) -> bool) -> Option<usize> {
        let found: Vec<usize> =
            self.alive(side).into_iter().filter(|&i| Some(i) != exclude && cond(&self.ms[i])).collect();
        if found.is_empty() { None } else { Some(self.rng.choice(&found)) }
    }

    fn buff(&mut self, i: usize, atk: i64, hp: i64) {
        let m = &mut self.ms[i];
        m.atk += atk;
        m.hp += hp;
        m.max_hp += hp;
        if atk != 0 {
            self.on_attack_changed(i);
        }
    }

    // -------------------------------------------------------------- damage
    /// Deals damage; returns the damage beyond the target's health (for Wildfire). Divine Shield
    /// absorbs all of it.
    fn damage(&mut self, target: usize, amount: i64, source: Option<usize>) -> i64 {
        if amount <= 0 || self.ms[target].dead {
            return 0;
        }
        if self.ms[target].ds {
            self.ms[target].ds = false;
            self.on_shield_lost(target);
            return 0;
        }
        let before = self.ms[target].hp;
        self.ms[target].hp -= amount;
        if let Some(s) = source {
            if self.ms[s].venom || self.ms[s].poison {
                self.ms[target].hp = self.ms[target].hp.min(0);
                self.ms[s].venom = false; // Venomous works once
            }
            if self.ms[target].hp <= 0 && self.ms[target].killer.is_none() {
                self.ms[target].killer = Some(s);
            }
            if self.ms[s].has_race("DEMON") {
                let side = self.ms[s].side;
                for h in self.alive(side) {
                    if self.ms[h].base == "BG33_155" && h != s {
                        let g = self.ms[h].g();
                        self.buff(h, 2 * g, 2 * g);
                    }
                }
            }
        }
        (amount - before).max(0)
    }

    // -------------------------------------------------------------- one hit
    fn attack(&mut self, side: usize, att: usize, target: Option<usize>, rally: bool) -> Option<usize> {
        let enemy = 1 - side;
        let target = match target.filter(|&t| !self.ms[t].dead) {
            Some(t) => t,
            None => {
                let targets = self.alive(enemy);
                if targets.is_empty() {
                    return None;
                }
                let taunts: Vec<usize> = targets.iter().copied().filter(|&i| self.ms[i].taunt).collect();
                let pool = if taunts.is_empty() { targets } else { taunts };
                let visible: Vec<usize> = pool.iter().copied().filter(|&i| !self.ms[i].stealth).collect();
                let visible = if visible.is_empty() { pool } else { visible };
                self.rng.choice(&visible)
            }
        };
        self.ms[att].stealth = false;
        if rally {
            self.before_attack(side, att, target);
        }
        if self.ms[att].dead || self.ms[target].dead {
            return Some(target);
        }
        let mut victims = vec![target];
        if self.ms[att].cleave {
            // Blade Collector: also hits the neighbours
            let eb = &self.sides[enemy];
            if let Some(i) = eb.iter().position(|&x| x == target) {
                victims = eb[i.saturating_sub(1)..(i + 2).min(eb.len())]
                    .iter()
                    .copied()
                    .filter(|&v| !self.ms[v].dead)
                    .collect();
            }
        }
        let hit_back = self.ms[target].atk;
        let mut excess = 0;
        for v in victims {
            let e = self.damage(v, self.ms[att].atk, Some(att));
            if v == target {
                excess = e;
            }
        }
        self.damage(att, hit_back, Some(target));
        self.after_attack(side, att, target, excess);
        Some(target)
    }

    // -------------------------------------------------------------- deaths
    /// Removes the dead (the side attacking first first, left to right) and plays Deathrattles and Reborn.
    fn resolve_deaths(&mut self, first: usize) {
        loop {
            let mut dead = Vec::new();
            for s in [first, 1 - first] {
                for &i in &self.sides[s] {
                    if self.ms[i].hp <= 0 && !self.ms[i].dead {
                        dead.push((s, i));
                    }
                }
            }
            if dead.is_empty() {
                return;
            }
            for &(_, i) in &dead {
                self.ms[i].dead = true;
            }
            for (side, i) in dead {
                let pos = self.remove(i);
                self.on_death(side, i, pos);
            }
        }
    }

    fn run(&mut self, first: usize) {
        let mut side = first;
        let mut attacks = 0;
        while attacks < MAX_ATTACKS {
            if !self.any_alive(0) || !self.any_alive(1) {
                break;
            }
            let Some(att) = self.next_attacker(side) else {
                if self.next_attacker(1 - side).is_none() {
                    break; // nobody can attack any more: tie
                }
                side = 1 - side;
                continue;
            };
            let swings = match self.ms[att].windfury {
                w if w >= 3 => 4,
                0 => 1,
                _ => 2,
            };
            for _ in 0..swings {
                if self.ms[att].dead || !self.any_alive(1 - side) || self.ms[att].atk <= 0 {
                    break;
                }
                if let Some(p) = self.pos(att) {
                    self.ptr[side] = p + 1;
                }
                self.attack(side, att, None, true);
                attacks += 1;
                self.resolve_deaths(side);
            }
            side = 1 - side;
        }
    }

    fn next_attacker(&self, side: usize) -> Option<usize> {
        let b = &self.sides[side];
        let n = b.len();
        if n == 0 {
            return None;
        }
        let start = self.ptr[side] % n;
        (0..n).map(|k| b[(start + k) % n]).find(|&i| self.ms[i].atk > 0 && !self.ms[i].dead)
    }

    /// Minions left on each side.
    fn left(&self, side: usize) -> Vec<&M> {
        self.sides[side].iter().map(|&i| &self.ms[i]).filter(|m| !m.dead).collect()
    }

    // ================================================================ card effects
    fn on_attack_changed(&mut self, i: usize) {
        let m = &mut self.ms[i];
        if m.base == "BG35_814" && !m.flag && m.atk >= 6 {
            // Scarlet Survivor
            m.flag = true;
            m.ds = true;
        }
    }

    fn on_shield_lost(&mut self, i: usize) {
        if self.ms[i].base == "BG34_405" {
            self.ms[i].taunt = false; // Relentless Deflector
        }
    }

    fn on_summon(&mut self, side: usize, m: usize) {
        for o in self.alive(side) {
            if o == m {
                continue;
            }
            let (ob, og, golden) = (self.ms[o].base.as_str(), self.ms[o].g(), self.ms[o].golden);
            match ob {
                "BG26_802" if self.ms[m].has_race("BEAST") => {
                    // Banana Slamma
                    self.ms[m].atk *= if golden { 3 } else { 2 };
                }
                "BG35_602" if self.ms[m].has_race("BEAST") => {
                    // Lurking Leviathan
                    self.ms[m].atk += 3 * og;
                }
                "BG34_322" => {
                    // Stalwart Kodo
                    let left = *self.kodo_left.entry(o).or_insert(3);
                    if left > 0 {
                        self.kodo_left.insert(o, left - 1);
                        let (a, h) = (self.ms[o].atk * og, self.ms[o].max_hp * og);
                        self.buff(m, a, h);
                    }
                }
                _ => {}
            }
        }
        self.on_attack_changed(m);
    }

    fn before_attack(&mut self, side: usize, att: usize, target: usize) {
        // Rally: "whenever this attacks"
        if let Some(r) = rally_of(&self.ms[att].base) {
            let times = if r.golden_in_text() { 1 } else { self.ms[att].g() };
            for _ in 0..times {
                self.rally(r, side, att, target);
            }
        }
        for o in self.alive(side) {
            if o == att {
                continue;
            }
            let g = self.ms[o].g();
            match self.ms[o].base.as_str() {
                "BG29_816" if self.ms[att].has_race("DRAGON") => self.buff(att, 3 * g, g), // Roaring Recruiter
                "BG33_430" => {
                    // Prodigious Tusker
                    let gem = self.gem[side];
                    self.buff(att, gem.0 * g, gem.1 * g);
                }
                _ => {}
            }
            if self.ms[o].base == "BG36_211" && self.ms[att].has_race("BEAST") {
                self.cage_gnawer(side, g); // Cage Gnawer
            }
        }
        if self.ms[att].base == "BG36_211" && self.ms[att].has_race("BEAST") {
            let g = self.ms[att].g();
            self.cage_gnawer(side, g);
        }
    }

    fn cage_gnawer(&mut self, side: usize, g: i64) {
        for m in self.alive(side) {
            if self.ms[m].has_race("BEAST") {
                self.buff(m, 2 * g, g);
            }
        }
    }

    fn neighbours(&self, side: usize, i: usize) -> Vec<usize> {
        let b = &self.sides[side];
        let mut near = Vec::new();
        if i > 0 {
            near.push(b[i - 1]);
        }
        if i + 1 < b.len() {
            near.push(b[i + 1]);
        }
        near
    }

    fn after_attack(&mut self, side: usize, att: usize, target: usize, excess: i64) {
        let enemy = 1 - side;
        if self.ms[att].base == "BGS_126" && excess > 0 && self.ms[target].hp <= 0 {
            // Wildfire Elemental
            if let Some(i) = self.sides[enemy].iter().position(|&x| x == target) {
                let mut near: Vec<usize> =
                    self.neighbours(enemy, i).into_iter().filter(|&m| !self.ms[m].dead).collect();
                if !near.is_empty() && !self.ms[att].golden {
                    near = vec![self.rng.choice(&near)];
                }
                for m in near {
                    self.damage(m, excess, Some(att));
                }
            }
        }
        if self.ms[att].base == "BG36_102" && !self.ms[att].dead {
            // De-volition-ist
            let enemies = self.alive(enemy);
            if let Some(top) = enemies.iter().map(|&m| self.ms[m].hp).max() {
                let best: Vec<usize> = enemies.into_iter().filter(|&m| self.ms[m].hp == top).collect();
                let t = self.rng.choice(&best);
                let amount = self.ms[att].atk * self.ms[att].g();
                self.damage(t, amount, Some(att));
            }
        }
        for o in self.alive(side) {
            let g = self.ms[o].g();
            if self.ms[o].base == "BG36_209" {
                // Ravaging Scorpid
                self.beetle[side].0 += 4 * g;
                self.beetle[side].1 += 4 * g;
            }
            if self.ms[o].base == "BG36_208" && rally_of(&self.ms[att].base).is_some() && o != att {
                // Deathstrider
                let dr = self.alive(side).into_iter().find(|&m| deathrattle_of(&self.ms[m].base).is_some());
                for _ in 0..g {
                    if let Some(dr) = dr {
                        if let (Some(p), Some(kind)) = (self.pos(dr), deathrattle_of(&self.ms[dr].base)) {
                            self.deathrattle(kind, side, dr, p + 1);
                        }
                    }
                }
            }
        }
    }

    fn on_death(&mut self, side: usize, m: usize, pos: usize) {
        self.deaths[side] += 1;
        let base = self.ms[m].base.clone();
        let mg = self.ms[m].g();
        // Leeroy: destroys its killer
        if base == "BG23_318" {
            if let Some(k) = self.ms[m].killer.filter(|&k| !self.ms[k].dead) {
                self.ms[k].hp = 0;
            }
        }
        // Rot Hide Gnoll, Eternal Knight
        for o in self.alive(side) {
            let g = self.ms[o].g();
            if self.ms[o].base == "BG25_013" {
                self.buff(o, g, 0);
            }
            if base == "BG25_008" && self.ms[o].base == "BG25_008" {
                self.buff(o, 4 * g, 2 * g);
            }
        }
        if base == "BG25_008" {
            self.knight_bonus[side] += 1;
        }
        // Avenge (X): every X friendly deaths
        for o in self.alive(side) {
            let Some((n, kind)) = avenge_of(&self.ms[o].base) else { continue };
            let mut k = self.avenge.get(&o).copied().unwrap_or(0) + 1;
            if k >= n {
                k = 0;
                for _ in 0..self.ms[o].g() {
                    self.avenge_effect(kind, side, o);
                }
            }
            self.avenge.insert(o, k);
        }
        // Deathrattle (Titus Rivendare = once more, golden = twice more)
        if let Some(kind) = deathrattle_of(&base) {
            let alive = self.alive(side);
            let times = 1 + alive.iter().filter(|&&o| self.ms[o].base == "BG25_354").map(|&o| self.ms[o].g()).sum::<i64>();
            for _ in 0..times {
                self.deathrattle(kind, side, m, pos);
                for o in self.alive(side) {
                    if self.ms[o].base == "BG35_342" {
                        // Falling Sky Golem
                        let g = self.ms[o].g();
                        self.buff(o, 4 * g, 2 * g);
                    }
                }
            }
        }
        // Reborn: comes back with 1 Health, without its buffs
        if self.ms[m].reborn {
            let card_id = self.ms[m].card_id.clone();
            let mut r = self.token(&card_id, side);
            r.hp = 1;
            r.max_hp = 1;
            r.reborn = false;
            r.tier = self.ms[m].tier;
            if base == "BG25_008" {
                // Eternal Knight: keeps its aura
                let n = self.ctx.knights[side] + self.knight_bonus[side];
                r.atk = (4 + 4 * n) * mg;
            }
            if let Some(r) = self.summon(side, r, Some(pos), true) {
                for o in self.alive(side) {
                    if o == r {
                        continue;
                    }
                    let g = self.ms[o].g();
                    match self.ms[o].base.as_str() {
                        "BG36_514" => {
                            // Barrier Banshee
                            self.ms[o].ds = true;
                            self.buff(o, 8 * g, 8 * g);
                        }
                        "BG36_515" => {
                            // Snazzy Phantom
                            let undead = self.alive(side).into_iter().filter(|&u| self.ms[u].has_race("UNDEAD")).last();
                            if let Some(u) = undead {
                                let a = self.ms[r].atk * g;
                                self.buff(u, a, a);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // Deity: after 4 friendly Aberrations died, it wakes up
        if self.ms[m].has_race("ABERRATION") && !base.starts_with("BGFYM") {
            self.dead_aberrations[side].push(m);
            self.aberration_deaths[side] += 1;
            if self.aberration_deaths[side] >= 4 && !self.deity_done[side] && self.ctx.deity_id.is_some() {
                self.deity_done[side] = true;
                self.awaken_deity(side);
            }
        }
        if base == "BGFYM_011" {
            // Y'Shaarj
            let firsts: Vec<usize> = self.dead_aberrations[side].iter().copied().take((2 * mg) as usize).collect();
            for a in firsts {
                let (cid, atk, hp) = (self.ms[a].card_id.clone(), self.ms[a].atk, self.ms[a].max_hp);
                let t = self.token(&cid, side).stats(atk, hp);
                self.summon(side, t, Some(pos), false);
            }
        }
    }

    fn buff_deity(&mut self, side: usize, atk: i64, hp: i64) {
        match self.alive(side).into_iter().find(|&m| self.ms[m].base.starts_with("BGFYM")) {
            Some(d) => self.buff(d, atk, hp),
            None => {
                self.deity_bonus[side].0 += atk;
                self.deity_bonus[side].1 += hp;
            }
        }
    }

    fn awaken_deity(&mut self, side: usize) {
        let Some(deity_id) = self.ctx.deity_id.clone() else { return };
        let (atk, hp) = self.ctx.deity_stats[side];
        let (atk, hp) = (atk + self.deity_bonus[side].0, hp + self.deity_bonus[side].1);
        let t = self.token(&deity_id, side).stats(atk, hp);
        let Some(d) = self.summon(side, t, Some(0), true) else { return };
        if deity_id == "BGFYM_000" {
            // C'Thun: shares its stats
            let others: Vec<usize> = self.alive(side).into_iter().filter(|&m| m != d).collect();
            if others.is_empty() {
                return;
            }
            for _ in 0..atk.max(0) {
                let o = self.rng.choice(&others);
                self.buff(o, 1, 0);
            }
            for _ in 0..hp.max(0) {
                let o = self.rng.choice(&others);
                self.buff(o, 0, 1);
            }
        }
    }

    // ---------------------------------------------------------------- Deathrattles
    fn deathrattle(&mut self, kind: Dr, side: usize, m: usize, pos: usize) {
        let g = self.ms[m].g();
        let golden = self.ms[m].golden;
        match kind {
            Dr::Beetles => self.beetles(side, g, pos),
            Dr::Skitterer => {
                self.beetle[side].0 += 5 * g;
                self.beetle[side].1 += 5 * g;
                self.beetles(side, g, pos);
            }
            Dr::Summon(card_id, n, golden_id) => {
                let (cid, count) = match golden_id {
                    Some(gid) => (if golden { gid } else { card_id }, n),
                    None => (card_id, n * g),
                };
                for _ in 0..count {
                    let t = self.token(cid, side);
                    self.summon(side, t, Some(pos), true);
                }
            }
            Dr::Tentacles => {
                for _ in 0..g {
                    let mut t = self.token("", side).stats(0, 2);
                    t.taunt = true;
                    t.races = race_bit("ABERRATION");
                    self.summon(side, t, Some(pos), true);
                }
                for o in self.alive(side) {
                    self.buff(o, g, 0);
                }
            }
            Dr::BuffRandom(a, h, race) => {
                if let Some(o) = self.pick(side, None, |x| x.has_race(race)) {
                    self.buff(o, a * g, h * g);
                }
            }
            Dr::BuffAll(a, h, race) => self.buff_all(side, a * g, h * g, race),
            Dr::Lobster => {
                let (a, h) = self.lobster[side];
                if let Some(o) = self.pick(side, None, |x| x.has_race("BEAST")) {
                    self.buff(o, a * g, h * g);
                }
            }
            Dr::Mummifier => {
                for _ in 0..g {
                    if let Some(o) = self.pick(side, None, |x| x.has_race("UNDEAD") && !x.reborn) {
                        self.ms[o].reborn = true;
                    }
                }
            }
            Dr::Ghastcoiler => {
                for _ in 0..2 * g {
                    if self.ctx.deathrattle_pool.is_empty() {
                        break;
                    }
                    let i = self.rng.below(self.ctx.deathrattle_pool.len());
                    let t = self.token(&self.ctx.deathrattle_pool[i], side);
                    self.summon(side, t, Some(pos), true);
                }
            }
            Dr::Nadina => {
                let mut dragons: Vec<usize> =
                    self.alive(side).into_iter().filter(|&o| self.ms[o].has_race("DRAGON") && !self.ms[o].ds).collect();
                self.rng.shuffle(&mut dragons);
                for o in dragons.into_iter().take((3 * g) as usize) {
                    self.ms[o].ds = true;
                }
            }
            Dr::Plaguerunner => {
                self.undead[side] += 2 * g;
                self.buff_all(side, 2 * g, 0, Some("UNDEAD"));
            }
            Dr::DeityBuff(a, h) => self.buff_deity(side, a * g, h * g),
            Dr::Knight => {
                let bonus = self.ctx.knights[side] + self.knight_bonus[side];
                let cid = if golden { "BG25_008_G" } else { "BG25_008" };
                let t = self.token(cid, side).stats((4 + 4 * bonus) * g, (2 + 2 * bonus) * g);
                self.summon(side, t, Some(pos), true);
            }
        }
    }

    fn beetles(&mut self, side: usize, n: i64, pos: usize) {
        let (a, h) = self.beetle[side];
        for _ in 0..n {
            let mut t = self.token(BEETLE, side).stats(a, h);
            t.races = race_bit("BEAST");
            self.summon(side, t, Some(pos), true);
        }
    }

    fn buff_all(&mut self, side: usize, atk: i64, hp: i64, race: Option<&str>) {
        for o in self.alive(side) {
            if race.is_none_or(|r| self.ms[o].has_race(r)) {
                self.buff(o, atk, hp);
            }
        }
    }

    // ---------------------------------------------------------------- Rally
    fn rally(&mut self, kind: Rally, side: usize, m: usize, target: usize) {
        let g = self.ms[m].g();
        let after = |f: &Self| f.pos(m).map(|p| p + 1);
        match kind {
            Rally::Summon(card_id, golden_id) => {
                let cid = if self.ms[m].golden { golden_id } else { card_id };
                let t = self.token(cid, side);
                let p = after(self);
                self.summon(side, t, p, true);
            }
            Rally::Gain(a, h) => self.buff(m, a, h),
            Rally::Wolf => {
                for o in self.alive(side) {
                    if o != m {
                        self.buff(o, 4, 1);
                    }
                }
            }
            Rally::Bonker => {
                let gem = self.gem[side];
                for o in self.alive(side) {
                    if o != m {
                        self.buff(o, gem.0, gem.1);
                    }
                }
            }
            Rally::Underdog => {
                let a = self.ms[target].atk;
                self.buff(m, a, 0);
            }
            Rally::Sindorei => {
                self.ms[target].reborn = false;
                self.ms[target].taunt = false;
            }
            Rally::Bile => {
                if let Some(o) = self.pick(side, Some(m), |x| x.has_race("MURLOC") && !x.venom) {
                    self.ms[o].venom = true;
                }
            }
            Rally::Brood => self.ms[m].ds = true,
            Rally::Ravager => {
                let enemy = 1 - side;
                let Some(i) = self.sides[enemy].iter().position(|&x| x == target) else { return };
                let near = self.neighbours(enemy, i);
                let mut hits = vec![target];
                if self.ms[m].golden {
                    hits.extend(near);
                } else if !near.is_empty() {
                    hits.push(self.rng.choice(&near));
                }
                for o in hits {
                    let a = self.ms[m].atk;
                    self.damage(o, a, Some(m));
                }
            }
            Rally::Gems(n) => {
                let gem = self.gem[side];
                self.buff(m, gem.0 * n, gem.1 * n);
            }
            Rally::Refiner => {
                self.gem[side].0 += 1;
                self.gem[side].1 += 2;
            }
            Rally::Dragonbreath => {
                let sp = self.ctx.spell[side];
                let (a, h) = (3 + sp.0, 2 + sp.1);
                for o in self.alive(side) {
                    let times = 1 + i64::from(self.ms[o].has_race("DRAGON")) + i64::from(self.ms[o].ds);
                    self.buff(o, a * times, h * times);
                }
            }
            Rally::LastOne => {
                let mask = self.alive(side).iter().fold(0u16, |acc, &o| acc | self.ms[o].races) & !ALL;
                for (bit, race) in RACES.iter().enumerate() {
                    if mask & (1 << bit) == 0 {
                        continue;
                    }
                    if let Some(o) = self.pick(side, None, |x| x.has_race(race)) {
                        self.buff(o, 15 * g, 15 * g);
                    }
                }
            }
            Rally::Jailbird => {
                let k = if self.ms[m].golden { 2 } else { 1 };
                let t = self.token("", side).stats(self.ms[m].atk * k, self.ms[m].hp * k);
                let p = after(self);
                if let Some(golem) = self.summon(side, t, p, true) {
                    if !self.ms[target].dead {
                        self.attack(side, golem, Some(target), false);
                        self.resolve_deaths(side);
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------- Avenge
    fn avenge_effect(&mut self, kind: Avenge, side: usize, m: usize) {
        match kind {
            Avenge::Deflector => {
                self.ms[m].ds = true;
                self.ms[m].taunt = true;
            }
            Avenge::Tycoon => {
                let bonus = self.ctx.knights[side] + self.knight_bonus[side];
                let t = self.token("BG25_008", side).stats(4 + 4 * bonus, 2 + 2 * bonus);
                let p = self.pos(m).map(|p| p + 1);
                if let Some(k) = self.summon(side, t, p, true) {
                    if self.any_alive(1 - side) {
                        self.attack(side, k, None, true);
                    }
                }
            }
        }
    }
}

// --------------------------------------------------------------------------------- effect tables
#[derive(Debug, Clone, Copy)]
enum Dr {
    Beetles,
    Skitterer,
    /// card, how many, golden card (None: twice as many when golden).
    Summon(&'static str, i64, Option<&'static str>),
    Tentacles,
    BuffRandom(i64, i64, &'static str),
    BuffAll(i64, i64, Option<&'static str>),
    Lobster,
    Mummifier,
    Ghastcoiler,
    Nadina,
    Plaguerunner,
    DeityBuff(i64, i64),
    Knight,
}

fn deathrattle_of(base: &str) -> Option<Dr> {
    Some(match base {
        "BG31_803" => Dr::Beetles,                                             // Buzzing Vermin
        "BG31_801" => Dr::Beetles,                                             // Forest Rover
        "BG36_209" => Dr::Beetles,                                             // Ravaging Scorpid
        "BG31_809" => Dr::Skitterer,                                           // Turquoise Skitterer
        "BG29_611" => Dr::Summon("BG_BOT_312t", 1, Some("TB_BaconUps_032t")), // Cord Puller
        "BG28_300" => Dr::Summon("BG_ICC_026t", 2, None),                      // Harmless Bonehead
        "BG30_125" => Dr::Summon("BG_ICC_026t", 3, None),                      // Cadaver Caretaker
        "BG25_010" => Dr::Summon("BG25_010t", 1, None),                        // Handless Forsaken
        "BG35_604" => Dr::Summon("BG19_010", 2, Some("BG19_010_G")),           // Sewer Lord
        "BG19_010" => Dr::Summon("BG19_010t", 1, Some("BG19_010_Gt")),         // Sewer Rat
        "BG36_116" => Dr::Tentacles,                                           // Underrot Spawn
        "BG25_022" => Dr::BuffRandom(1, 2, "UNDEAD"),                          // Scarlet Skull
        "BG36_202" => Dr::Lobster,                                             // Tasty Lobster
        "BG28_309" => Dr::Mummifier,                                           // Mummifier
        "BGS_008" => Dr::Ghastcoiler,                                          // Ghastcoiler
        "BGS_018" => Dr::BuffAll(7, 7, Some("BEAST")),                         // Goldrinn
        "BG33_828" => Dr::BuffAll(6, 6, None),                                 // Ship Master Eudora
        "BG34_690" => Dr::Plaguerunner,                                        // Plaguerunner
        "BGS_040" => Dr::Nadina,                                               // Nadina the Red
        "BG25_009" => Dr::Knight,                                              // Eternal Summoner
        "BG36_113" => Dr::DeityBuff(2, 1),                                     // Drifting Sacrifice
        "BG36_318" => Dr::DeityBuff(2, 1), // Faceless Converter (without its spell bonus)
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy)]
enum Rally {
    /// card, golden card.
    Summon(&'static str, &'static str),
    Gain(i64, i64),
    Wolf,
    Bonker,
    Underdog,
    Sindorei,
    Bile,
    Brood,
    Ravager,
    Gems(i64),
    Refiner,
    Dragonbreath,
    LastOne,
    Jailbird,
}

impl Rally {
    /// The golden version is already handled inside the effect (don't play it twice).
    fn golden_in_text(self) -> bool {
        matches!(self, Rally::Summon(..) | Rally::Ravager | Rally::Jailbird)
    }
}

fn rally_of(base: &str) -> Option<Rally> {
    Some(match base {
        "BG36_200" => Rally::Summon("BG36_200t", "BG36_200t"), // Flittering Bat
        "BG29_888" => Rally::Gain(2, 0),                       // Glim Guardian
        "BG33_886" => Rally::Gems(1),                          // Tusked Camper
        "BG36_207" => Rally::Wolf,                             // Wolf Pup
        "BG20_104" => Rally::Bonker,                           // Bonker
        "BG34_604" => Rally::Underdog,                         // Heroic Underdog
        "BG25_016" => Rally::Sindorei,                         // Sin'dorei Straight Shot
        "BG33_318" => Rally::Bile,                             // Bile Spitter
        "BG36_210" => Rally::Summon("BG36_202", "BG36_202_G"), // Hoarding Hyena
        "BG36_849" => Rally::Brood,                            // Heroic Broodmother
        "BG27_017" => Rally::Ravager,                          // Obsidian Ravager
        "BG33_883" => Rally::Gems(4),                          // Razorfen Vineweaver
        "BG33_885" => Rally::Refiner,                          // Sanguine Refiner
        "BG36_241" => Rally::Dragonbreath,                     // Crimson Vindicator
        "BG34_320" => Rally::LastOne,                          // The Last One Standing
        "BG36_333" => Rally::Jailbird,                         // Jailbird Juggernaut
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy)]
enum Avenge {
    Deflector,
    Tycoon,
}

fn avenge_of(base: &str) -> Option<(i64, Avenge)> {
    match base {
        "BG34_405" => Some((3, Avenge::Deflector)), // Relentless Deflector
        "BG34_403" => Some((5, Avenge::Tycoon)),    // Eternal Tycoon
        _ => None,
    }
}

/// Effects coded elsewhere in `Fight` (not reported as unknown).
const HANDLED_ELSEWHERE: &[&str] = &[
    "BG35_814", "BG34_405", "BG26_802", "BG35_602", "BG34_322", "BG29_816", "BG33_430", "BG36_211", "BGS_126", "BG36_102",
    "BG36_209", "BG36_208", "BG23_318", "BG25_013", "BG25_008", "BG25_354", "BG36_514", "BG36_515", "BG33_155", "BG26_817",
    "BGFYM_011", "BGFYM_000", "BG35_342",
];
/// "In combat" effects that don't change the fight's result (hand, Tavern, hero after the fight).
const NO_COMBAT_EFFECT: &[&str] =
    &["BG26_174", "BG29_300", "BG36_243", "BG36_362", "BG26_523", "BG23_017", "BG27_016", "BG32_873", "BG36_733"];
/// Words that, in a card's text, mean "this acts during combat".
const COMBAT_WORDS: &[&str] = &[
    "Deathrattle", "Rally", "Avenge", "attacks", "this combat", "takes damage", "dies", "is Reborn", "summon", "Summon",
    "loses Divine Shield", "deals damage",
];
/// ... except these effects that change nothing in the fight (cards added to hand, Tavern, next game).
const SHOP_ONLY: &[&str] = &[
    "Get a", "Get an", "Get 2", "Get 3", "Tavern", "Lockbox", "Blood Gems give", "next turn", "Refresh", "spells give",
    "Bounty", "Chromadrake", "Repair Job", "Fodder",
];

/// A card whose effect matters in combat but is not simulated (the user is warned).
pub fn is_unknown_effect(card: Option<&Card>) -> bool {
    let Some(card) = card else { return false };
    let id = card.id.as_str();
    if deathrattle_of(id).is_some()
        || rally_of(id).is_some()
        || avenge_of(id).is_some()
        || HANDLED_ELSEWHERE.contains(&id)
        || NO_COMBAT_EFFECT.contains(&id)
    {
        return false;
    }
    let text = card.text.as_str();
    if !COMBAT_WORDS.iter().any(|w| text.contains(w)) {
        return false;
    }
    if text.contains("Start of Combat") && !["Deathrattle", "Rally", "Avenge", "attacks"].iter().any(|w| text.contains(w)) {
        return false; // already played when the picture was taken
    }
    let before_get = text.split("Get").next().unwrap_or("");
    if SHOP_ONLY.iter().any(|w| text.contains(w)) && !["Summon", "Give"].iter().any(|w| before_get.contains(w)) {
        return false;
    }
    true
}

// ------------------------------------------------------------------------------------- simulation
/// One side's "this game" bonuses: Beetle stats, Tasty Lobster... The log's counters (hidden player
/// enchantments, see Game::player_counters) first, else what the cards show.
fn side_ctx(board: &[MinionSnap], counters: &Counters, db: &CardDb) -> ((i64, i64), (i64, i64)) {
    let mut beetle = match counters.beetle {
        Some((a, h)) => (2 + a, 2 + h),
        None => (2, 2),
    };
    let mut lobster = (2, 1);
    for m in board {
        let Some(base) = db.base(&m.card_id) else { continue };
        let (a, h) = m.data;
        if ["BG31_803", "BG31_801", "BG31_809", "BG36_209"].contains(&base.id.as_str())
            && a >= 2
            && h >= 2
            && counters.beetle.is_none()
        {
            beetle = (beetle.0.max(a), beetle.1.max(h));
        }
        if base.id == "BG36_202" && a > 0 {
            lobster = (a, h);
        }
    }
    (beetle, lobster)
}

fn deathrattle_pool(db: &CardDb) -> Vec<String> {
    db.pool_minions(false)
        .into_iter()
        .filter(|c| deathrattle_of(&c.id).is_some() && c.id != "BGS_008")
        .map(|c| c.id.clone())
        .collect()
}

/// Everything about one fight to simulate.
#[derive(Debug, Clone, PartialEq)]
pub struct SimInput {
    pub mine: Vec<MinionSnap>,
    pub theirs: Vec<MinionSnap>,
    pub my_tier: i64,
    pub their_tier: i64,
    pub my_health: Option<i64>,
    pub their_health: Option<i64>,
    /// 0 = I attack first, 1 = they do, None = the game's rule (more minions, else random).
    pub first: Option<u8>,
    /// Card id of the game's Deity (BGFYM_000 C'Thun, BGFYM_011 Y'Shaarj).
    pub deity: Option<String>,
    /// Deity (attack, health) for me, for them.
    pub deity_stats: [(i64, i64); 2],
    /// Game counters of each side (beetle, knights, undead: see Game::player_counters).
    pub counters: [Counters; 2],
    /// What one Blood Gem gives, each side.
    pub gem: [(i64, i64); 2],
    /// Tavern spell bonus, each side.
    pub spell: [(i64, i64); 2],
    /// Damage cap of the fight (BACON_COMBAT_DAMAGE_CAP, goes up with turns).
    pub cap: Option<i64>,
}

impl Default for SimInput {
    fn default() -> Self {
        SimInput {
            mine: vec![],
            theirs: vec![],
            my_tier: 1,
            their_tier: 1,
            my_health: None,
            their_health: None,
            first: None,
            deity: None,
            deity_stats: [(1, 1), (1, 1)],
            counters: [Counters::default(), Counters::default()],
            gem: [(1, 1), (1, 1)],
            spell: [(0, 0), (0, 0)],
            cap: None,
        }
    }
}

/// Results in percent (win + tie + loss = 100), damage when winning / losing, averages over all fights.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SimResult {
    pub runs: u32,
    pub win: f64,
    pub tie: f64,
    pub loss: f64,
    pub dealt: f64,
    pub taken: f64,
    pub avg_dealt: f64,
    pub avg_taken: f64,
    /// Chances the opponent dies.
    pub lethal: f64,
    /// Chances I die.
    pub lethal_me: f64,
    /// Names of the board cards whose combat effect is not simulated.
    pub unknown: Vec<String>,
    pub first: Option<u8>,
}

#[derive(Default, Clone, Copy)]
struct Tally {
    n: u32,
    win: u32,
    tie: u32,
    loss: u32,
    dealt: i64,
    taken: i64,
    lethal: u32,
    lethal_me: u32,
}

impl Tally {
    fn add(self, o: Tally) -> Tally {
        Tally {
            n: self.n + o.n,
            win: self.win + o.win,
            tie: self.tie + o.tie,
            loss: self.loss + o.loss,
            dealt: self.dealt + o.dealt,
            taken: self.taken + o.taken,
            lethal: self.lethal + o.lethal,
            lethal_me: self.lethal_me + o.lethal_me,
        }
    }
}

/// Replays the fight `runs` times (or for `seconds` at most) and returns the odds.
/// The same seed gives the same result.
pub fn simulate(input: &SimInput, runs: u32, seconds: f64, seed: Option<u64>) -> SimResult {
    let db = cards::db();
    simulate_with(&db, input, runs, seconds, seed)
}

pub fn simulate_with(db: &CardDb, input: &SimInput, runs: u32, seconds: f64, seed: Option<u64>) -> SimResult {
    let (b0, l0) = side_ctx(&input.mine, &input.counters[0], db);
    let (b1, l1) = side_ctx(&input.theirs, &input.counters[1], db);
    let ctx = Ctx {
        db,
        beetle: [b0, b1],
        lobster: [l0, l1],
        deathrattle_pool: deathrattle_pool(db),
        deity_id: input.deity.clone(),
        deity_stats: input.deity_stats,
        knights: [input.counters[0].knights, input.counters[1].knights],
        undead: [input.counters[0].undead, input.counters[1].undead],
        gem: input.gem,
        spell: input.spell,
    };
    let protos = [
        input.mine.iter().map(|d| M::new(d, 0, db)).collect::<Vec<_>>(),
        input.theirs.iter().map(|d| M::new(d, 1, db)).collect::<Vec<_>>(),
    ];
    let seed = seed.unwrap_or_else(|| Rng::from_entropy().next_u64());
    let deadline = Instant::now() + Duration::from_secs_f64(seconds.max(0.0));
    let chunks = runs.div_ceil(CHUNK).max(1);
    let total = (0..chunks)
        .into_par_iter()
        .map(|c| {
            // The first chunk always runs, so there is always a result.
            if c > 0 && Instant::now() > deadline {
                return Tally::default();
            }
            let mut rng = Rng::new(seed ^ (u64::from(c) + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let count = CHUNK.min(runs.saturating_sub(c * CHUNK)).max(if c == 0 { 1 } else { 0 });
            let mut t = Tally::default();
            for _ in 0..count {
                one_run(&protos, &ctx, input, &mut rng, &mut t);
            }
            t
        })
        .reduce(Tally::default, Tally::add);
    let n = total.n.max(1);
    let pct = |k: u32| 100.0 * f64::from(k) / f64::from(n);
    let unknown: BTreeSet<String> = input
        .mine
        .iter()
        .chain(&input.theirs)
        .filter_map(|m| db.base(&m.card_id))
        .filter(|c| is_unknown_effect(Some(c)))
        .map(|c| c.name.clone())
        .collect();
    SimResult {
        runs: n,
        win: pct(total.win),
        tie: pct(total.tie),
        loss: pct(total.loss),
        dealt: if total.win > 0 { total.dealt as f64 / f64::from(total.win) } else { 0.0 },
        taken: if total.loss > 0 { total.taken as f64 / f64::from(total.loss) } else { 0.0 },
        avg_dealt: total.dealt as f64 / f64::from(n),
        avg_taken: total.taken as f64 / f64::from(n),
        lethal: pct(total.lethal),
        lethal_me: pct(total.lethal_me),
        unknown: unknown.into_iter().collect(),
        first: input.first,
    }
}

fn one_run(protos: &[Vec<M>; 2], ctx: &Ctx, input: &SimInput, rng: &mut Rng, t: &mut Tally) {
    let who = match input.first {
        Some(f) => usize::from(f.min(1)),
        None => {
            let (a, b) = (input.mine.len(), input.theirs.len());
            if a > b {
                0
            } else if b > a {
                1
            } else {
                usize::from(rng.coin())
            }
        }
    };
    let mut f = Fight::new(protos, ctx, rng);
    f.run(who);
    let (left0, left1) = (f.left(0), f.left(1));
    let capped = |d: i64| match input.cap {
        Some(c) if c != 0 => d.min(c),
        _ => d,
    };
    t.n += 1;
    if !left0.is_empty() && left1.is_empty() {
        t.win += 1;
        let d = capped(input.my_tier + left0.iter().map(|m| m.tier).sum::<i64>());
        t.dealt += d;
        if input.their_health.is_some_and(|h| d >= h) {
            t.lethal += 1;
        }
    } else if !left1.is_empty() && left0.is_empty() {
        t.loss += 1;
        let d = capped(input.their_tier + left1.iter().map(|m| m.tier).sum::<i64>());
        t.taken += d;
        if input.my_health.is_some_and(|h| d >= h) {
            t.lethal_me += 1;
        }
    } else {
        t.tie += 1;
    }
}

/// Card id of a Deity by its name.
pub fn deity_id(name: &str) -> Option<&'static str> {
    match name {
        "C'Thun" => Some("BGFYM_000"),
        "Y'Shaarj" => Some("BGFYM_011"),
        _ => None,
    }
}

#[cfg(test)]
mod tests;

/// What to simulate for a fight frozen by game.rs (None if nobody attacked).
pub fn fight_input(game: &crate::game::Game, f: &crate::game::Fight) -> Option<SimInput> {
    let fr = f.frozen.as_ref()?;
    Some(SimInput {
        mine: fr.my_board.clone(),
        theirs: f.board.clone().unwrap_or_default(),
        my_tier: fr.my_tier,
        their_tier: fr.opp_tier,
        my_health: game.my_health(),
        their_health: f.opponent.and_then(|o| game.player_health(o)),
        first: fr.first,
        deity: game.deity().and_then(|d| deity_id(&d)).map(str::to_string),
        deity_stats: [fr.deity_stats.0.unwrap_or((1, 1)), fr.deity_stats.1.unwrap_or((1, 1))],
        counters: [fr.counters.0.clone(), fr.counters.1.clone()],
        gem: [fr.gem, (1, 1)],
        spell: [fr.spell, (0, 0)],
        cap: fr.cap,
    })
}

/// Odds of a frozen fight (computed once, kept in the fight).
pub fn fight_odds(game: &mut crate::game::Game, index: usize, (runs, seconds): (u32, f64)) -> Option<SimResult> {
    let f = game.fights.get(index)?;
    if f.sim.is_none() {
        let input = fight_input(game, f)?;
        let r = simulate(&input, runs, seconds, None);
        game.fights[index].sim = Some(r);
    }
    game.fights[index].sim.clone()
}
