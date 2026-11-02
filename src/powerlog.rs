//! Power.log reader: turns the lines of Hearthstone's log into entities and tags.
//!
//! - Hearthstone writes every change of a game in Power.log (one line = one event).
//! - `PowerLogParser` reads these lines one by one (`feed_line`) and keeps a `Game` up to date.
//! - Only "GameState.DebugPrintPower" lines are read: the "PowerTaskList" lines repeat the same events
//!   (a bit later, for the animations) and would create duplicates.

use std::sync::LazyLock;

use regex::Regex;

use crate::game::Game;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid regex")
}

// Useful lines. Example:
// "D 12:00:41.4016168 GameState.DebugPrintPower() -     TAG_CHANGE Entity=10 tag=TURN value=1"
static PREFIX: LazyLock<Regex> = LazyLock::new(|| re(r"^D ([\d:.]+) GameState\.DebugPrint(Power|Game)\(\) - (.*)$"));
// A tag line inside a block (FULL_ENTITY, SHOW_ENTITY...): "tag=ZONE value=PLAY"
static TAG: LazyLock<Regex> = LazyLock::new(|| re(r"^tag=(\S+) value=(\S*)"));
// "FULL_ENTITY - Creating ID=31 CardID=TB_BaconShop_HERO_PH"
static FULL_CREATE: LazyLock<Regex> = LazyLock::new(|| re(r"^FULL_ENTITY - Creating ID=(\d+) CardID=(\S*)"));
// "FULL_ENTITY - Updating [entityName=... id=31 ...] CardID=..."
static FULL_UPDATE: LazyLock<Regex> = LazyLock::new(|| re(r"^FULL_ENTITY - Updating (.+) CardID=(\S*)"));
// A hidden card shown: "SHOW_ENTITY - Updating Entity=254 CardID=BG29_813e"
static SHOW: LazyLock<Regex> = LazyLock::new(|| re(r"^SHOW_ENTITY - Updating Entity=(.+) CardID=(\S*)"));
// A card that changes (e.g. placeholder hero -> real hero): same shape as SHOW_ENTITY
static CHANGE: LazyLock<Regex> = LazyLock::new(|| re(r"^CHANGE_ENTITY - Updating Entity=(.+) CardID=(\S*)"));
// "HIDE_ENTITY - Entity=[...] tag=ZONE value=SETASIDE"
static HIDE: LazyLock<Regex> = LazyLock::new(|| re(r"^HIDE_ENTITY - Entity=(.+) tag=(\S+) value=(\S*)"));
// "TAG_CHANGE Entity=Player#1234 tag=TURN value=1 "
static TAG_CHANGE: LazyLock<Regex> = LazyLock::new(|| re(r"^TAG_CHANGE Entity=(.+) tag=(\S+) value=(\S*)"));
// "GameEntity EntityID=10"
static GAME_ENTITY: LazyLock<Regex> = LazyLock::new(|| re(r"^GameEntity EntityID=(\d+)"));
// "Player EntityID=11 PlayerID=4 GameAccountId=[hi=144115198130930503 lo=28027830]"
static PLAYER: LazyLock<Regex> =
    LazyLock::new(|| re(r"^Player EntityID=(\d+) PlayerID=(\d+) GameAccountId=\[hi=(\d+) lo=(\d+)\]"));
// Game header: "PlayerID=4, PlayerName=Player#1234" and "GameType=GT_BATTLEGROUNDS"
static PLAYER_NAME: LazyLock<Regex> = LazyLock::new(|| re(r"^PlayerID=(\d+), PlayerName=(.*)$"));
static GAME_TYPE: LazyLock<Regex> = LazyLock::new(|| re(r"^GameType=(\S+)"));
// "BLOCK_START BlockType=ATTACK Entity=[...] ..."
static BLOCK: LazyLock<Regex> =
    LazyLock::new(|| re(r"^BLOCK_START BlockType=(\S+)(?: Entity=(\[.*?\bid=\d+[^\]]*\]|\S+))?"));
// "[entityName=Thorim, Stormlord id=91 zone=PLAY ...]" -> 91
static BRACKET_ID: LazyLock<Regex> = LazyLock::new(|| re(r"\bid=(\d+)\b"));

/// Reads Power.log line by line and updates the current game.
pub struct PowerLogParser {
    /// Every game seen (only the current one in live mode); the last one is the current game.
    games: Vec<Game>,
    keep_games: bool,
    /// Entity whose "tag=..." lines are being read.
    current: Option<i64>,
    /// Time of the last line read.
    time: String,
    /// Goes up by one at each new game.
    pub started: u64,
}

impl Default for PowerLogParser {
    fn default() -> Self {
        PowerLogParser::new(true)
    }
}

impl PowerLogParser {
    /// `keep_games` = false in live mode: only the current game stays in memory.
    pub fn new(keep_games: bool) -> PowerLogParser {
        PowerLogParser { games: Vec::new(), keep_games, current: None, time: String::new(), started: 0 }
    }

    pub fn game(&self) -> Option<&Game> {
        self.games.last()
    }

    pub fn game_mut(&mut self) -> Option<&mut Game> {
        self.games.last_mut()
    }

    pub fn games(&self) -> &[Game] {
        &self.games
    }

    pub fn into_games(self) -> Vec<Game> {
        self.games
    }

    /// Reads ONE raw line of the file. Useless lines are ignored.
    pub fn feed_line(&mut self, line: &str) {
        let line = line.trim_end_matches(['\r', '\n']);
        // Quick filter before the regex: most lines are PowerTaskList ones.
        if !line.starts_with("D ") || !line.contains("GameState.DebugPrint") {
            return;
        }
        let Some(m) = PREFIX.captures(line) else { return };
        self.time = m[1].to_string();
        let body = m[3].trim();
        if &m[2] == "Game" {
            self.read_game_header(body);
        } else {
            self.read_power(body);
        }
    }

    pub fn feed_lines<'a>(&mut self, lines: impl IntoIterator<Item = &'a str>) {
        for line in lines {
            self.feed_line(line);
        }
    }

    /// Reads a whole log file.
    pub fn feed_file(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        let bytes = std::fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes);
        self.feed_lines(text.lines());
        Ok(())
    }

    fn read_game_header(&mut self, body: &str) {
        let Some(g) = self.games.last_mut() else { return };
        if let Some(m) = GAME_TYPE.captures(body) {
            g.game_type = Some(m[1].to_string()); // e.g. GT_BATTLEGROUNDS
        } else if let Some(m) = PLAYER_NAME.captures(body) {
            // PlayerID -> display name (BattleTag for me, Bob's name for the innkeeper)
            if let Ok(pid) = m[1].parse() {
                g.set_player_name(pid, &m[2]);
            }
        }
    }

    fn read_power(&mut self, body: &str) {
        // 1) A "tag=X value=Y" line completes the entity of the current block.
        if let Some(m) = TAG.captures(body) {
            if let (Some(eid), Some(g)) = (self.current, self.games.last_mut()) {
                g.set_tag(eid, &m[1], &m[2]);
            }
            return;
        }
        // Any other line ends the current block of tags.
        self.current = None;
        if body == "CREATE_GAME" {
            self.start_game();
            return;
        }
        if self.games.is_empty() {
            return; // events before a game starts: ignored
        }
        if let Some(m) = TAG_CHANGE.captures(body) {
            if let Some(eid) = self.resolve(&m[1]) {
                self.game_mut().expect("a game").set_tag(eid, &m[2], &m[3]);
            }
        } else if let Some(m) = FULL_CREATE.captures(body) {
            if let Ok(eid) = m[1].parse() {
                self.current = Some(self.game_mut().expect("a game").create_entity(eid, &m[2]));
            }
        } else if let Some(m) =
            SHOW.captures(body).or_else(|| CHANGE.captures(body)).or_else(|| FULL_UPDATE.captures(body))
        {
            if let Some(eid) = self.resolve(&m[1]) {
                if !m[2].is_empty() {
                    // the card is known now (or changed)
                    self.game_mut().expect("a game").set_card_id(eid, &m[2]);
                }
                self.current = Some(eid); // the next "tag=" lines complete it
            }
        } else if let Some(m) = HIDE.captures(body) {
            if let Some(eid) = self.resolve(&m[1]) {
                self.game_mut().expect("a game").set_tag(eid, &m[2], &m[3]);
            }
        } else if let Some(m) = BLOCK.captures(body) {
            let eid = m.get(2).and_then(|e| self.resolve(e.as_str()));
            self.game_mut().expect("a game").on_block(&m[1], eid);
        } else if let Some(m) = GAME_ENTITY.captures(body) {
            if let Ok(eid) = m[1].parse() {
                let g = self.game_mut().expect("a game");
                g.create_entity(eid, "");
                g.game_entity_id = Some(eid);
                self.current = Some(eid);
            }
        } else if let Some(m) = PLAYER.captures(body) {
            if let (Ok(eid), Ok(pid)) = (m[1].parse(), m[2].parse()) {
                let g = self.game_mut().expect("a game");
                g.create_entity(eid, "");
                // hi=0: a made-up account = Bob (the innkeeper). Otherwise a real account.
                g.add_player(pid, eid, &m[3] != "0");
                self.current = Some(eid);
            }
        }
        // BLOCK_END / META_DATA: not needed.
    }

    /// CREATE_GAME: a new game starts from scratch.
    fn start_game(&mut self) {
        let mut g = Game::new();
        g.start_time = self.time.chars().take(8).collect(); // "12:00:41": tells games apart
        if !self.keep_games {
            self.games.clear();
        }
        self.games.push(g);
        self.started += 1;
    }

    /// The entity a reference names: a number ("254"), brackets ("[entityName=... id=91 ...]"),
    /// "GameEntity", or a player name ("Player#1234").
    fn resolve(&self, reference: &str) -> Option<i64> {
        let g = self.games.last()?;
        let r = reference.trim();
        let id = if !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()) {
            r.parse().ok()
        } else if r.starts_with('[') {
            BRACKET_ID.captures(r).and_then(|m| m[1].parse().ok())
        } else if r == "GameEntity" {
            g.game_entity_id
        } else {
            return g.entity_by_player_name(r);
        };
        id.filter(|id| g.entity(*id).is_some())
    }
}
