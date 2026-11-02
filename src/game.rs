//! One game's state: entities, tags, and the Battlegrounds reading of that state.
//!
//! Words:
//! - Entity: any object of the game (the game itself, a player, a hero, a minion, a hero power, an
//!   enchantment...). Each entity has a number (id) and tags.
//! - Tag: a "name=value" property, e.g. ZONE=PLAY (on the board), CONTROLLER=4 (whose),
//!   PLAYER_TECH_LEVEL=3 (a hero's tavern tier), CARDRACE=BEAST.
//! - Bob: the innkeeper. His minions in ZONE=PLAY outside combat are the shop.
//!
//! The parser (powerlog.rs) calls `create_entity` and `set_tag`; every tag goes through `on_tag`, which
//! keeps the deductions up to date (lobby tribes from the shop, fights, actions per turn...).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::cards::{self, Card};
use crate::combat::{MinionSnap, SimResult, snapshot_minion};

const BONUS_TAGS: [&str; 4] = [
    "TAVERN_SPELL_ATTACK_INCREASE",
    "TAVERN_SPELL_HEALTH_INCREASE",
    "BACON_BLOODGEMBUFFATKVALUE",
    "BACON_BLOODGEMBUFFHEALTHVALUE",
];

/// A tag value: a number ("3") or a name ("PLAY").
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Text(Box<str>),
}

impl Value {
    pub fn parse(raw: &str) -> Value {
        match raw.parse::<i64>() {
            Ok(n) => Value::Int(n),
            Err(_) => Value::Text(raw.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entity {
    pub id: i64,
    /// "" while the card is hidden.
    pub card_id: String,
    tags: HashMap<Box<str>, Value>,
}

impl Entity {
    fn new(id: i64, card_id: &str) -> Entity {
        Entity { id, card_id: card_id.to_string(), tags: HashMap::new() }
    }

    pub fn get(&self, tag: &str) -> Option<&Value> {
        self.tags.get(tag)
    }

    /// The tag as a number, or `default` when missing (or not a number).
    pub fn int_or(&self, tag: &str, default: i64) -> i64 {
        match self.tags.get(tag) {
            Some(Value::Int(n)) => *n,
            _ => default,
        }
    }

    pub fn int(&self, tag: &str) -> i64 {
        self.int_or(tag, 0)
    }

    pub fn opt_int(&self, tag: &str) -> Option<i64> {
        match self.tags.get(tag) {
            Some(Value::Int(n)) => Some(*n),
            _ => None,
        }
    }

    pub fn text(&self, tag: &str) -> Option<&str> {
        match self.tags.get(tag) {
            Some(Value::Text(t)) => Some(t),
            _ => None,
        }
    }

    pub fn is(&self, tag: &str, value: &str) -> bool {
        self.text(tag) == Some(value)
    }

    /// Set to something other than 0 or "".
    pub fn truthy(&self, tag: &str) -> bool {
        match self.tags.get(tag) {
            Some(Value::Int(n)) => *n != 0,
            Some(Value::Text(t)) => !t.is_empty(),
            None => false,
        }
    }

    /// The normal (not golden) card of this entity.
    pub fn card(&self) -> Option<Arc<Card>> {
        if self.card_id.is_empty() {
            return None;
        }
        cards::db().base(&self.card_id).cloned()
    }

    pub fn name(&self) -> String {
        match self.card() {
            Some(c) => c.name.clone(),
            None if !self.card_id.is_empty() => self.card_id.clone(),
            None => format!("#{}", self.id),
        }
    }

    pub fn is_minion(&self) -> bool {
        self.is("CARDTYPE", "MINION")
    }

    /// PREMIUM=1: the golden version (a triple).
    pub fn is_golden(&self) -> bool {
        self.truthy("PREMIUM")
    }

    /// Health left: HEALTH - DAMAGE + ARMOR (heroes).
    pub fn health_left(&self) -> i64 {
        self.int("HEALTH") - self.int("DAMAGE") + self.int("ARMOR")
    }
}

#[derive(Debug, Clone)]
pub struct Player {
    pub entity_id: i64,
    /// hi=0 in GameAccountId: a made-up account, i.e. Bob.
    pub is_real_account: bool,
}

/// Game-long counters the game keeps on hidden enchantments of a player (beetles, knights, undead).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Counters {
    /// Extra attack / health of this player's Beetles (None = no counter seen).
    pub beetle: Option<(i64, i64)>,
    /// Eternal Knights dead this game.
    pub knights: i64,
    /// "Your Undead have +X Attack this game".
    pub undead: i64,
}

/// What was frozen at the first attack of a fight (both boards, tiers, bonuses).
#[derive(Debug, Clone)]
pub struct Frozen {
    pub my_board: Vec<MinionSnap>,
    pub my_tier: i64,
    pub opp_tier: i64,
    /// Who attacked first: 0 = me, 1 = them, None = unknown.
    pub first: Option<u8>,
    pub counters: (Counters, Counters),
    pub deity_stats: (Option<(i64, i64)>, Option<(i64, i64)>),
    pub gem: (i64, i64),
    pub spell: (i64, i64),
    pub cap: Option<i64>,
}

/// One of my fights.
#[derive(Debug, Clone)]
pub struct Fight {
    pub turn: i64,
    /// The opponent's PLAYER_ID.
    pub opponent: Option<i64>,
    entered: Vec<i64>,
    /// The opponent's board at the first attack; empty if nobody attacked.
    pub board: Option<Vec<MinionSnap>>,
    pub my_hp: Option<i64>,
    pub opp_hp: Option<i64>,
    /// Set when the boards were frozen at the first attack.
    pub frozen: Option<Frozen>,
    /// 'W', 'L' or 'T' once the fight is over.
    pub result: char,
    pub damage: i64,
    /// The simulator's odds for this fight (computed once).
    pub sim: Option<SimResult>,
}

/// My bonuses (counters shown by trackers): Tavern spells and Blood Gems.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bonuses {
    /// Extra attack / health my Tavern spells give.
    pub spell: (i64, i64),
    /// What one Blood Gem gives (+1/+1 base, plus the BACON_BLOODGEMBUFF*VALUE tags).
    pub gem: (i64, i64),
}

/// Hidden enchantments holding a player's game-long counters (like trackers read them for simulators).
/// During a fight, the opponent's are copied onto the Bob player.
fn counter_kind(card_id: &str) -> Option<&'static str> {
    match card_id {
        "BG31_808pe" => Some("beetle"),
        "BG25_008pe" => Some("knights"),
        "BG25_011pe" => Some("undead"),
        _ => None,
    }
}

#[derive(Default)]
pub struct Game {
    entities: Vec<Entity>,
    index: HashMap<i64, usize>,
    /// PlayerID -> player, in the order they appeared.
    players: Vec<(i64, Player)>,
    /// PlayerID -> display name, in the order they appeared.
    player_names: Vec<(i64, String)>,
    /// e.g. "GT_BATTLEGROUNDS".
    pub game_type: Option<String>,
    /// Time of CREATE_GAME in the log, e.g. "12:00:41".
    pub start_time: String,
    pub game_entity_id: Option<i64>,
    /// Tribe -> number of single-tribe minions seen in the shop, in the order first seen.
    shop_race_counts: Vec<(String, i64)>,
    seen_in_shop: HashSet<i64>,
    /// Tribe pairs of dual-tribe minions seen in the shop.
    dual_race_seen: Vec<Vec<String>>,
    /// Entities that carried a BACON_SUBSET_* tag (neutrals tied to a tribe).
    pub subset_tags_seen: HashSet<i64>,
    pub fights: Vec<Fight>,
    fight: Option<Fight>,
    /// PLAYER_ID -> (attack, health) of their Deity the last time it was seen.
    pub deity_seen: HashMap<i64, (i64, i64)>,
    bonus_tags: HashMap<&'static str, i64>,
    /// Turn -> number of actions (buy, sell, play, roll, level, hero power...).
    pub actions_by_turn: HashMap<i64, i64>,
    /// I conceded (PLAYSTATE = CONCEDED).
    pub conceded: bool,
    /// In Duos both teammates are real accounts: the one whose hero choices we can see is me.
    local_override: Option<i64>,
}

impl Game {
    pub fn new() -> Game {
        Game::default()
    }

    // ------------------------------------------------------------------ filling
    pub fn create_entity(&mut self, eid: i64, card_id: &str) -> i64 {
        match self.index.get(&eid) {
            Some(&i) => {
                if !card_id.is_empty() {
                    self.entities[i].card_id = card_id.to_string();
                }
            }
            None => {
                self.index.insert(eid, self.entities.len());
                self.entities.push(Entity::new(eid, card_id));
            }
        }
        eid
    }

    pub fn set_card_id(&mut self, eid: i64, card_id: &str) {
        if let Some(&i) = self.index.get(&eid) {
            self.entities[i].card_id = card_id.to_string();
        }
    }

    pub fn set_tag(&mut self, eid: i64, tag: &str, raw: &str) {
        let Some(&i) = self.index.get(&eid) else { return };
        self.entities[i].tags.insert(tag.into(), Value::parse(raw));
        self.on_tag(eid, tag);
    }

    pub fn add_player(&mut self, pid: i64, entity_id: i64, is_real_account: bool) {
        let p = Player { entity_id, is_real_account };
        match self.players.iter_mut().find(|(id, _)| *id == pid) {
            Some(slot) => slot.1 = p,
            None => self.players.push((pid, p)),
        }
    }

    pub fn set_player_name(&mut self, pid: i64, name: &str) {
        match self.player_names.iter_mut().find(|(id, _)| *id == pid) {
            Some(slot) => slot.1 = name.to_string(),
            None => self.player_names.push((pid, name.to_string())),
        }
    }

    pub fn entity(&self, eid: i64) -> Option<&Entity> {
        self.index.get(&eid).map(|&i| &self.entities[i])
    }

    /// Every entity, in the order the log created them.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.iter()
    }

    pub fn entity_by_player_name(&self, name: &str) -> Option<i64> {
        for (pid, pname) in &self.player_names {
            if pname == name {
                if let Some(p) = self.player(*pid) {
                    return Some(p.entity_id);
                }
            }
        }
        None
    }

    pub fn player(&self, pid: i64) -> Option<&Player> {
        self.players.iter().find(|(id, _)| *id == pid).map(|(_, p)| p)
    }

    fn on_tag(&mut self, eid: i64, tag: &str) {
        let Some(ent) = self.entity(eid) else { return };
        let controller = ent.opt_int("CONTROLLER");
        let int_value = ent.int(tag);
        let conceded = ent.is(tag, "CONCEDED");
        if tag.starts_with("BACON_SUBSET_") {
            self.subset_tags_seen.insert(eid);
        }
        if let Some(&bonus) = BONUS_TAGS.iter().find(|t| **t == tag) {
            if let Some(me) = self.local_player_id() {
                let mine = controller == Some(me) || self.player(me).is_some_and(|p| p.entity_id == eid);
                if mine {
                    self.bonus_tags.insert(bonus, int_value);
                }
            }
        }
        if tag == "NUM_OPTIONS_PLAYED_THIS_TURN" || tag == "PLAYSTATE" {
            let me = self.local_player_id().and_then(|pid| self.player(pid)).map(|p| p.entity_id);
            if me == Some(eid) {
                if tag == "PLAYSTATE" {
                    self.conceded = self.conceded || conceded;
                } else if !self.in_combat() && self.turn() > 0 {
                    // The game counts each choice made during the turn (back to 0 next turn).
                    let turn = self.turn();
                    let slot = self.actions_by_turn.entry(turn).or_insert(0);
                    *slot = (*slot).max(int_value);
                }
            }
        }
        let ent = self.entity(eid).expect("entity exists");
        if Some(eid) == self.game_entity_id && (tag == "TURN" || tag == "STATE") {
            if tag == "TURN" {
                self.on_turn();
            } else if self.fight.is_some() && ent.is("STATE", "COMPLETE") {
                self.end_fight(); // last fight: the game stops before the next turn
            }
            return;
        }
        if tag == "ZONE" && ent.is("ZONE", "PLAY") {
            if let Some(f) = self.fight.as_mut() {
                f.entered.push(eid); // minion put in play during this fight
            }
        }
        if matches!(tag, "ZONE" | "CONTROLLER" | "CARDTYPE" | "BACON_HERO_CAN_BE_DRAFTED") {
            self.check_local_player(eid);
        }
        if !matches!(tag, "ZONE" | "CONTROLLER" | "CARDTYPE") || self.seen_in_shop.contains(&eid) {
            return;
        }
        let Some(bob) = self.bob_player_id() else { return };
        let ent = self.entity(eid).expect("entity exists");
        if !ent.is_minion() || !ent.is("ZONE", "PLAY") || ent.opt_int("CONTROLLER") != Some(bob) || self.in_combat() {
            return;
        }
        // Only pool minions tell which tribes are in the lobby.
        let Some(card) = ent.card() else { return };
        if !card.is_pool_minion {
            return;
        }
        self.seen_in_shop.insert(eid);
        // An Amalgam (ALL) says nothing.
        let races: Vec<String> = card.races.iter().filter(|r| *r != "ALL").cloned().collect();
        if races.len() == 1 {
            // A single tribe is proof.
            match self.shop_race_counts.iter_mut().find(|(r, _)| *r == races[0]) {
                Some(slot) => slot.1 += 1,
                None => self.shop_race_counts.push((races[0].clone(), 1)),
            }
        } else if !races.is_empty() {
            // Dual tribe (e.g. Demon/Dragon): ONE of the two is enough for it to be in the shop.
            // Keep the pair to decide later.
            self.dual_race_seen.push(races);
        }
    }

    /// Duos: both teammates are real accounts. My hero choices are shown in the log (card ids), so the
    /// real player who holds visible draftable heroes in hand is me.
    fn check_local_player(&mut self, eid: i64) {
        if self.local_override.is_some() || self.players.iter().filter(|(_, p)| p.is_real_account).count() < 2 {
            return;
        }
        let Some(e) = self.entity(eid) else { return };
        if !e.is("CARDTYPE", "HERO") || !e.truthy("BACON_HERO_CAN_BE_DRAFTED") || !e.is("ZONE", "HAND") {
            return;
        }
        if e.card_id.is_empty() || e.card_id.contains("HERO_PH") {
            return;
        }
        let Some(c) = e.opt_int("CONTROLLER") else { return };
        if self.player(c).is_some_and(|p| p.is_real_account) {
            self.local_override = Some(c);
        }
    }

    // ------------------------------------------------------------------ fights
    // During a fight, Bob "plays" my opponent's board: copies made at the start of the fight. We note
    // these copies (ids put in play during the fight), then freeze the board at the first ATTACK block,
    // before deaths and Deathrattle summons.
    fn on_turn(&mut self) {
        if self.in_combat() {
            self.start_fight();
        } else if self.fight.is_some() {
            self.end_fight();
        }
    }

    fn start_fight(&mut self) {
        let me = self.player_entity(self.local_player_id());
        let opp = me.and_then(|e| e.opt_int("NEXT_OPPONENT_PLAYER_ID"));
        self.fight = Some(Fight {
            turn: self.turn(),
            opponent: opp,
            entered: Vec::new(),
            board: None,
            my_hp: self.my_health(),
            opp_hp: opp.and_then(|o| self.player_health(o)),
            frozen: None,
            result: 'T',
            damage: 0,
            sim: None,
        });
    }

    /// Called by the parser on each BLOCK_START (e.g. "ATTACK"), with the block's entity.
    pub fn on_block(&mut self, block_type: &str, eid: Option<i64>) {
        if block_type == "ATTACK" && self.fight.as_ref().is_some_and(|f| f.board.is_none()) {
            self.freeze_fight(eid);
        }
    }

    /// Picture of both boards at the first hit: for the Lobby list and for the simulator.
    /// Start of Combat effects have already happened at that point (their buffs are in the stats).
    fn freeze_fight(&mut self, attacker: Option<i64>) {
        let db = cards::db();
        let bob = self.bob_player_id();
        let entered = self.fight.as_ref().map(|f| f.entered.clone()).unwrap_or_default();
        let mut seen = HashSet::new();
        let mut board: Vec<&Entity> = Vec::new();
        for eid in entered {
            let Some(e) = self.entity(eid) else { continue };
            if !seen.insert(eid) || !e.is_minion() || e.opt_int("CONTROLLER") != bob || !e.is("ZONE", "PLAY") {
                continue;
            }
            board.push(e);
        }
        board.sort_by_key(|e| e.int("ZONE_POSITION"));
        let opp_board: Vec<MinionSnap> = board.iter().map(|e| snapshot_minion(e, &db)).collect();
        let my_board: Vec<MinionSnap> = self.my_board().iter().map(|e| snapshot_minion(e, &db)).collect();
        let opponent = self.fight.as_ref().and_then(|f| f.opponent);
        let opp_tier = opponent
            .and_then(|o| self.hero_of(o))
            .and_then(|h| h.opt_int("PLAYER_TECH_LEVEL"))
            .filter(|t| *t != 0)
            .unwrap_or(1);
        // Who attacks first: the controller of the first attacker (me, or Bob playing the opponent).
        let first = attacker
            .and_then(|a| self.entity(a))
            .map(|a| if a.opt_int("CONTROLLER") == self.local_player_id() { 0 } else { 1 });
        let local = self.local_player_id();
        let counters = (self.player_counters(local), self.player_counters(bob));
        let mine_deity = self.my_player_id().and_then(|p| self.deity_seen.get(&p).copied());
        let their_deity = opponent.and_then(|o| self.deity_seen.get(&o).copied());
        let bonuses = self.my_bonuses();
        let frozen = Frozen {
            my_board,
            my_tier: self.tavern_tier().filter(|t| *t != 0).unwrap_or(1),
            opp_tier,
            first,
            counters,
            deity_stats: (mine_deity, their_deity),
            gem: bonuses.gem,
            spell: bonuses.spell,
            cap: self.damage_cap(),
        };
        if let Some(f) = self.fight.as_mut() {
            f.board = Some(opp_board);
            f.frozen = Some(frozen);
        }
    }

    fn end_fight(&mut self) {
        let Some(mut f) = self.fight.take() else { return };
        if f.board.is_none() {
            f.board = Some(Vec::new()); // nobody attacked: empty (or unknown) board
        }
        // Deities awakened during this fight: keep their stats for the next simulations.
        let local = self.local_player_id();
        let my_pid = self.my_player_id();
        for eid in &f.entered {
            let Some(e) = self.entity(*eid) else { continue };
            if e.card_id.starts_with("BGFYM_") && e.is_minion() {
                let owner = if e.opt_int("CONTROLLER") == local { my_pid } else { f.opponent };
                if let Some(owner) = owner {
                    self.deity_seen.insert(owner, (e.int_or("ATK", 1), e.int_or("HEALTH", 1)));
                }
            }
        }
        f.entered = Vec::new();
        let my_loss = f.my_hp.unwrap_or(0) - self.my_health().unwrap_or(0);
        let opp_now = f.opponent.and_then(|o| self.player_health(o));
        let opp_loss = f.opp_hp.unwrap_or(0) - opp_now.unwrap_or(0);
        f.result = if my_loss > 0 { 'L' } else if opp_loss > 0 { 'W' } else { 'T' };
        f.damage = if my_loss > 0 { my_loss } else { opp_loss };
        self.fights.push(f);
    }

    /// Beetle bonus (+atk, +hp), Eternal Knights dead, +attack of Undead for this player.
    pub fn player_counters(&self, pid: Option<i64>) -> Counters {
        let mut found = Counters::default();
        let Some(p) = pid.and_then(|pid| self.player(pid)) else { return found };
        for e in &self.entities {
            let Some(kind) = counter_kind(&e.card_id) else { continue };
            if e.is("ZONE", "PLAY") && e.opt_int("ATTACHED") == Some(p.entity_id) {
                let (a, h) = (e.int("TAG_SCRIPT_DATA_NUM_1"), e.int("TAG_SCRIPT_DATA_NUM_2"));
                match kind {
                    "beetle" => found.beetle = Some((a, h)),
                    "knights" => found.knights = a,
                    _ => found.undead = a,
                }
            }
        }
        found
    }

    /// The fight going on, once both boards are frozen.
    pub fn current_fight(&self) -> Option<&Fight> {
        self.fight.as_ref().filter(|f| f.board.is_some())
    }

    pub fn current_fight_mut(&mut self) -> Option<&mut Fight> {
        self.fight.as_mut().filter(|f| f.board.is_some())
    }

    pub fn last_fight_against(&self, player_id: i64) -> Option<&Fight> {
        self.fights.iter().rev().find(|f| f.opponent == Some(player_id))
    }

    // ------------------------------------------------------------------ players
    pub fn is_battlegrounds(&self) -> bool {
        self.game_type.as_deref().is_some_and(|t| t.starts_with("GT_BATTLEGROUNDS"))
    }

    /// Duos (GT_BATTLEGROUNDS_DUO...): the pool also has the Duos cards.
    pub fn is_duos(&self) -> bool {
        self.game_type.as_deref().is_some_and(|t| t.contains("DUO"))
    }

    /// Me: the player with a real account (GameAccountId hi != 0); in Duos, see check_local_player.
    pub fn local_player_id(&self) -> Option<i64> {
        self.local_override.or_else(|| self.players.iter().find(|(_, p)| p.is_real_account).map(|(id, _)| *id))
    }

    pub fn bob_player_id(&self) -> Option<i64> {
        self.players.iter().find(|(_, p)| !p.is_real_account).map(|(id, _)| *id)
    }

    pub fn game_entity(&self) -> Option<&Entity> {
        self.game_entity_id.and_then(|id| self.entity(id))
    }

    fn player_entity(&self, pid: Option<i64>) -> Option<&Entity> {
        pid.and_then(|pid| self.player(pid)).and_then(|p| self.entity(p.entity_id))
    }

    // ------------------------------------------------------------------ Battlegrounds state
    /// Battlegrounds turn: the game's counter goes up by 2 each turn (recruit + combat).
    pub fn turn(&self) -> i64 {
        (self.game_entity().map(|g| g.int("TURN")).unwrap_or(0) + 1) / 2
    }

    /// During combat, Bob's minions in play are the opponent's, not the shop.
    pub fn in_combat(&self) -> bool {
        let t = self.game_entity().map(|g| g.int("TURN")).unwrap_or(0);
        t > 0 && t % 2 == 0
    }

    pub fn my_hero(&self) -> Option<&Entity> {
        let me = self.player_entity(self.local_player_id())?;
        let hero = self.entity(me.opt_int("HERO_ENTITY")?)?;
        if hero.card_id.contains("HERO_PH") {
            return None; // placeholder hero before the pick
        }
        Some(hero)
    }

    /// Heroes offered to pick from (before picking): my draftable heroes still in hand.
    /// A reroll changes the card of the same entity, so the list stays right.
    pub fn hero_options(&self) -> Vec<&Entity> {
        let me = self.local_player_id();
        let Some(player) = self.player_entity(me) else { return vec![] };
        // MULLIGAN_STATE goes to DEALING as soon as I clicked: the pick is made.
        if self.my_hero().is_some() || matches!(player.text("MULLIGAN_STATE"), Some("DEALING" | "WAITING" | "DONE")) {
            return vec![];
        }
        let mut found: Vec<&Entity> = self
            .entities
            .iter()
            .filter(|e| {
                e.is("CARDTYPE", "HERO")
                    && e.truthy("BACON_HERO_CAN_BE_DRAFTED")
                    && e.opt_int("CONTROLLER") == me
                    && e.is("ZONE", "HAND")
            })
            .collect();
        found.sort_by_key(|e| e.int("ZONE_POSITION"));
        found
    }

    pub fn tavern_tier(&self) -> Option<i64> {
        self.my_hero().map(|h| h.int_or("PLAYER_TECH_LEVEL", 1))
    }

    pub fn my_health(&self) -> Option<i64> {
        self.my_hero().map(Entity::health_left)
    }

    pub fn my_gold(&self) -> Option<i64> {
        let me = self.player_entity(self.local_player_id())?;
        Some(me.int("RESOURCES") - me.int("RESOURCES_USED") + me.int("TEMP_RESOURCES"))
    }

    fn minions(&self, controller: Option<i64>, zone: &str) -> Vec<&Entity> {
        let Some(c) = controller else { return vec![] };
        let mut found: Vec<&Entity> = self
            .entities
            .iter()
            .filter(|e| e.is_minion() && e.opt_int("CONTROLLER") == Some(c) && e.is("ZONE", zone))
            .collect();
        found.sort_by_key(|e| e.int("ZONE_POSITION"));
        found
    }

    pub fn my_board(&self) -> Vec<&Entity> {
        self.minions(self.local_player_id(), "PLAY")
    }

    pub fn my_hand(&self) -> Vec<&Entity> {
        let me = self.local_player_id();
        if me.is_none() {
            return vec![];
        }
        self.entities
            .iter()
            .filter(|e| {
                e.opt_int("CONTROLLER") == me
                    && e.is("ZONE", "HAND")
                    && matches!(e.text("CARDTYPE"), Some("MINION" | "BATTLEGROUND_SPELL"))
            })
            .collect()
    }

    pub fn shop(&self) -> Vec<&Entity> {
        if self.in_combat() { vec![] } else { self.minions(self.bob_player_id(), "PLAY") }
    }

    /// True once some single-tribe minion was seen in the shop.
    pub fn has_shop_races(&self) -> bool {
        !self.shop_race_counts.is_empty()
    }

    /// Sure tribes of the lobby: seen on a single-tribe minion in Bob's shop (most seen first).
    pub fn available_races(&self) -> Vec<String> {
        let mut counts = self.shop_race_counts.clone();
        counts.sort_by(|a, b| b.1.cmp(&a.1)); // stable: ties keep the order first seen
        counts.into_iter().take(crate::season::get().lobby_tribes).map(|(r, _)| r).collect()
    }

    /// Likely tribes not proven yet (from dual-tribe minions no sure tribe explains).
    pub fn possible_races(&self) -> Vec<String> {
        let sure: HashSet<&str> = self.shop_race_counts.iter().map(|(r, _)| r.as_str()).collect();
        if sure.len() >= crate::season::get().lobby_tribes {
            return vec![];
        }
        let mut out: Vec<String> = self
            .dual_race_seen
            .iter()
            .filter(|pair| !pair.iter().any(|r| sure.contains(r.as_str())))
            .flatten()
            .cloned()
            .collect();
        out.sort();
        out.dedup();
        out
    }

    pub fn races_complete(&self) -> bool {
        self.shop_race_counts.len() >= crate::season::get().lobby_tribes
    }

    /// The 8 heroes of the lobby: HERO entities with a leaderboard place, by place.
    pub fn lobby_heroes(&self) -> Vec<&Entity> {
        let mut heroes: Vec<&Entity> = self
            .entities
            .iter()
            .filter(|e| e.is("CARDTYPE", "HERO") && e.truthy("PLAYER_LEADERBOARD_PLACE") && !e.card_id.contains("HERO_PH"))
            .collect();
        // One player can have several hero entities (combat copy, transformed hero): keep the one in
        // play, else the newest (highest id).
        heroes.sort_by_key(|e| (e.is("ZONE", "PLAY"), e.id));
        let mut by_player: Vec<(i64, &Entity)> = Vec::new();
        fn put<'e>(by_player: &mut Vec<(i64, &'e Entity)>, h: &'e Entity) {
            let key = h.opt_int("PLAYER_ID").unwrap_or(h.id);
            match by_player.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = h,
                None => by_player.push((key, h)),
            }
        }
        for h in heroes {
            put(&mut by_player, h);
        }
        // My hero: always the one named by HERO_ENTITY (combat copies get stale).
        if let Some(mine) = self.my_hero() {
            put(&mut by_player, mine);
        }
        let mut out: Vec<&Entity> = by_player.into_iter().map(|(_, h)| h).collect();
        out.sort_by_key(|h| h.int("PLAYER_LEADERBOARD_PLACE"));
        out
    }

    /// Hero entity of the lobby player with this PLAYER_ID.
    pub fn hero_of(&self, player_id: i64) -> Option<&Entity> {
        self.lobby_heroes().into_iter().find(|h| h.opt_int("PLAYER_ID") == Some(player_id))
    }

    pub fn player_health(&self, player_id: i64) -> Option<i64> {
        self.hero_of(player_id).map(Entity::health_left)
    }

    /// My PLAYER_ID in the lobby (the one NEXT_OPPONENT_PLAYER_ID and fights refer to).
    pub fn my_player_id(&self) -> Option<i64> {
        self.my_hero().and_then(|h| h.opt_int("PLAYER_ID"))
    }

    pub fn next_opponent_id(&self) -> Option<i64> {
        self.player_entity(self.local_player_id()).and_then(|e| e.opt_int("NEXT_OPPONENT_PLAYER_ID"))
    }

    /// The game's Deity (e.g. "C'Thun"): tag BACON_GLOBAL_OLD_GOD_DBID of the game entity.
    pub fn deity(&self) -> Option<String> {
        let dbf = self.game_entity()?.opt_int("BACON_GLOBAL_OLD_GOD_DBID").filter(|d| *d != 0)?;
        cards::db().by_dbf.get(&dbf).map(|c| c.name.clone())
    }

    pub fn is_over(&self) -> bool {
        self.game_entity().is_some_and(|g| g.is("STATE", "COMPLETE"))
    }

    /// Damage cap of this turn's fight (BACON_COMBAT_DAMAGE_CAP), if enabled.
    pub fn damage_cap(&self) -> Option<i64> {
        let g = self.game_entity()?;
        if g.truthy("BACON_COMBAT_DAMAGE_CAP_ENABLED") { g.opt_int("BACON_COMBAT_DAMAGE_CAP").filter(|c| *c != 0) } else { None }
    }

    /// My hero powers in play (a hero can change power, or have two).
    pub fn my_hero_powers(&self) -> Vec<&Entity> {
        let me = self.local_player_id();
        let mut found: Vec<&Entity> = self
            .entities
            .iter()
            .filter(|e| {
                e.is("CARDTYPE", "HERO_POWER") && me.is_some() && e.opt_int("CONTROLLER") == me && e.is("ZONE", "PLAY") && !e.card_id.is_empty()
            })
            .collect();
        found.sort_by_key(|e| (e.int("ADDITIONAL_HERO_POWER_INDEX"), e.id));
        found
    }

    /// My permanent bonuses: the last value seen on an entity of mine (player, hero...).
    pub fn my_bonuses(&self) -> Bonuses {
        let t = |k: &str| self.bonus_tags.get(k).copied().unwrap_or(0);
        Bonuses {
            spell: (t("TAVERN_SPELL_ATTACK_INCREASE"), t("TAVERN_SPELL_HEALTH_INCREASE")),
            gem: (1 + t("BACON_BLOODGEMBUFFATKVALUE"), 1 + t("BACON_BLOODGEMBUFFHEALTHVALUE")),
        }
    }

    /// My tavern button in play (the game makes a new one each turn: the newest).
    fn my_button(&self, prefix: &str) -> Option<&Entity> {
        let me = self.local_player_id()?;
        self.entities
            .iter()
            .filter(|e| e.card_id.starts_with(prefix) && e.opt_int("CONTROLLER") == Some(me) && e.is("ZONE", "PLAY"))
            .max_by_key(|e| e.id)
    }

    /// Cost to level up now (button TB_BaconShopTechUp0X, 1 cheaper each turn); None at max tier.
    pub fn upgrade_cost(&self) -> Option<i64> {
        self.my_button("TB_BaconShopTechUp").map(|b| b.int("COST"))
    }

    /// Cost of a shop refresh (usually 1, 0 with some powers or effects).
    pub fn reroll_cost(&self) -> i64 {
        self.my_button("TB_BaconShop_8p_Reroll_Button").map(|b| b.int_or("COST", 1)).unwrap_or(1)
    }

    /// Trinkets offered now (turns 6 and 9): (entity, real price). The offer = my Trinkets in SETASIDE
    /// created by my "Lesser / Greater Trinket" button (or a power); the one picked becomes that button
    /// (CHANGE_ENTITY), the others go to REMOVEDFROMGAME. COST on the entity = discounted "pivot" price.
    pub fn trinket_offer(&self) -> Vec<(&Entity, i64)> {
        if self.in_combat() {
            return vec![];
        }
        let Some(me) = self.local_player_id() else { return vec![] };
        let db = cards::db();
        let mut found = Vec::new();
        for e in &self.entities {
            if !e.is("CARDTYPE", "BATTLEGROUND_TRINKET")
                || e.opt_int("CONTROLLER") != Some(me)
                || !e.is("ZONE", "SETASIDE")
                || !e.truthy("BACON_TRINKET")
                || e.card_id.is_empty()
            {
                continue;
            }
            let creator = e.opt_int("CREATOR").and_then(|c| self.entity(c));
            if !creator.is_some_and(|c| c.opt_int("CONTROLLER") == Some(me) && c.is("ZONE", "PLAY")) {
                continue; // copies seen in combat, other players' Trinkets...
            }
            let cost = e.opt_int("COST").unwrap_or_else(|| db.get(&e.card_id).map(|c| c.cost).unwrap_or(0));
            found.push((e, cost));
        }
        found.sort_by_key(|(e, _)| e.id);
        found
    }

    /// My bought Trinkets (unused "Lesser / Greater Trinket" buttons don't count).
    pub fn my_trinkets(&self) -> Vec<&Entity> {
        let me = self.local_player_id();
        if me.is_none() {
            return vec![];
        }
        self.entities
            .iter()
            .filter(|e| {
                e.is("CARDTYPE", "BATTLEGROUND_TRINKET")
                    && e.opt_int("CONTROLLER") == me
                    && e.is("ZONE", "PLAY")
                    && !e.card_id.is_empty()
                    && !e.card_id.starts_with("BG30_Trinket_")
            })
            .collect()
    }

    pub fn my_placement(&self) -> Option<i64> {
        self.my_hero().and_then(|h| h.opt_int("PLAYER_LEADERBOARD_PLACE")).filter(|p| *p != 0)
    }

    /// Readable summary of the state (console replay and tests).
    pub fn summary(&self) -> serde_json::Value {
        let names = |v: Vec<&Entity>| v.iter().map(|e| e.name()).collect::<Vec<_>>();
        serde_json::json!({
            "turn": self.turn(),
            "hero": self.my_hero().map(Entity::name),
            "tier": self.tavern_tier(),
            "health": self.my_health(),
            "gold": self.my_gold(),
            "board": self.my_board().iter().map(|e| e.name() + if e.is_golden() { " (G)" } else { "" }).collect::<Vec<_>>(),
            "hand": names(self.my_hand()),
            "shop": names(self.shop()),
            "races": self.available_races(),
            "races_complete": self.races_complete(),
            "possible_races": self.possible_races(),
            "deity": self.deity(),
            "placement": self.my_placement(),
            "over": self.is_over(),
        })
    }
}

impl Fight {
    /// A frozen fight made up for tests.
    #[doc(hidden)]
    pub fn for_tests(turn: i64, opp_tier: i64, board: Vec<MinionSnap>) -> Fight {
        Fight {
            turn,
            opponent: None,
            entered: Vec::new(),
            board: Some(board),
            my_hp: None,
            opp_hp: None,
            frozen: Some(Frozen {
                my_board: Vec::new(),
                my_tier: 1,
                opp_tier,
                first: None,
                counters: (Counters::default(), Counters::default()),
                deity_stats: (None, None),
                gem: (1, 1),
                spell: (0, 0),
                cap: None,
            }),
            result: 'T',
            damage: 0,
            sim: None,
        }
    }
}
