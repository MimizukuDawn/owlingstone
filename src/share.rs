//! Anonymous game sharing and player feedback (opt-in), to the project's Cloudflare API (server/).
//!
//! - `state()` / `set_enabled(on)`: the player's choice ("Share my games"), asked once, NO by default.
//!   A random install id (uuid4) is made at that moment; it is tied to no account.
//! - `game_summary(game, record)`: the summary of a finished game, reduced to the fields listed in
//!   PRIVACY.md (hero, Deity, tribes, place, comp, final board and fights: cards and stats). Never a
//!   BattleTag, an opponent's name, a file path or a log.
//! - `enqueue(...)` puts a summary or a feedback in a local queue (share_queue.jsonl); `flush()` sends the
//!   queue in ONE request (up to 20 games + 5 feedbacks) at the end of a game and at start: few requests,
//!   to stay in Cloudflare's free tier. Offline, the queue waits (30 days at most).
//! - `delete_remote()` deletes on the server everything this install sent, then forgets the id.
//! - `download_stats()` fetches stats.json (all players' stats) from GitHub once a day.

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::game::Game;
use crate::history::Record;
use crate::{VERSION, net, paths};

/// The API (Cloudflare Workers).
pub const API_URL: &str = "https://owlingstone-api.mimizuku-dawn.workers.dev";
pub const STATS_URL: &str = "https://github.com/owlingstone/owlingstone/releases/download/stats/stats.json";
const MAX_AGE: f64 = 30.0 * 86400.0;
const MAX_GAMES: usize = 20;
const MAX_FEEDBACK: usize = 5;
const MAX_QUEUE: usize = 200;
const STATS_LIMIT: u64 = 2_000_000;
pub const FEEDBACK_KINDS: [&str; 3] = ["idea", "bug", "other"];

/// One thread at a time reads-changes-writes the queue: a feedback typed during a send is never lost.
static QUEUE_LOCK: Mutex<()> = Mutex::new(());

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Sends a request: (method, url, body) -> HTTP status, or Err when offline.
pub type Send<'a> = &'a dyn Fn(&str, &str, Option<&Value>) -> Result<u16, String>;

pub fn http_send(method: &str, url: &str, body: Option<&Value>) -> Result<u16, String> {
    net::send(method, url, body, 20)
}

/// Where the files are and where to send (the defaults, or a test folder).
#[derive(Debug, Clone)]
pub struct Share {
    pub dir: PathBuf,
    pub api: String,
}

impl Default for Share {
    fn default() -> Self {
        Share { dir: paths::user_dir(), api: API_URL.to_string() }
    }
}

impl Share {
    fn state_path(&self) -> PathBuf {
        self.dir.join("sharing.json")
    }
    fn queue_path(&self) -> PathBuf {
        self.dir.join("share_queue.jsonl")
    }
    pub fn last_path(&self) -> PathBuf {
        self.dir.join("share_last.json")
    }
    fn stats_path(&self) -> PathBuf {
        self.dir.join("stats.json")
    }

    // ------------------------------------------------------------------ the player's choice
    pub fn state(&self) -> serde_json::Map<String, Value> {
        std::fs::read_to_string(self.state_path())
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    fn save(&self, s: &serde_json::Map<String, Value>) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.state_path(), Value::Object(s.clone()).to_string())
    }

    /// The player answered the question (yes or no).
    pub fn asked(&self) -> bool {
        self.state().get("enabled").is_some_and(|v| !v.is_null())
    }

    pub fn enabled(&self) -> bool {
        !self.api.is_empty() && self.state().get("enabled") == Some(&Value::Bool(true))
    }

    pub fn install_id(&self) -> String {
        let mut s = self.state();
        if let Some(id) = s.get("install").and_then(Value::as_str).filter(|i| !i.is_empty()) {
            return id.to_string();
        }
        let id = uuid::Uuid::new_v4().to_string();
        s.insert("install".into(), Value::String(id.clone()));
        let _ = self.save(&s);
        id
    }

    pub fn set_enabled(&self, on: bool) -> std::io::Result<()> {
        let mut s = self.state();
        s.insert("enabled".into(), Value::Bool(on));
        self.save(&s)?;
        if on {
            self.install_id();
        }
        Ok(())
    }

    // ------------------------------------------------------------------ local queue
    fn read_queue(&self) -> Vec<Value> {
        let now = now();
        let Ok(text) = std::fs::read_to_string(self.queue_path()) else { return vec![] };
        text.lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|i| i.is_object() && now - i.get("queued").and_then(Value::as_f64).unwrap_or(0.0) < MAX_AGE)
            .collect()
    }

    fn write_queue(&self, items: &[Value]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let text: String = items.iter().map(|i| i.to_string() + "\n").collect();
        let tmp = self.queue_path().with_extension("jsonl.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, self.queue_path())
    }

    fn enqueue(&self, kind: &str, data: Value) -> std::io::Result<()> {
        {
            let _lock = QUEUE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut items = self.read_queue();
            items.push(json!({"kind": kind, "data": data, "queued": now(), "qid": uuid::Uuid::new_v4().simple().to_string()}));
            let start = items.len().saturating_sub(MAX_QUEUE);
            self.write_queue(&items[start..])?;
        }
        if kind == "game" {
            std::fs::write(self.last_path(), serde_json::to_string_pretty(&data).unwrap_or_default())?;
        }
        Ok(())
    }

    /// End of a game: queues the summary if the player shares. Returns true if added.
    pub fn add_game(&self, game: &Game, record: &Record) -> bool {
        self.enabled() && self.enqueue("game", game_summary(game, record)).is_ok()
    }

    /// Feedback typed in the "Send feedback" window: sent with the next send (even without sharing).
    pub fn add_feedback(&self, kind: &str, text: &str) -> Result<(), String> {
        let text: String = text.trim().chars().take(1000).collect();
        if !FEEDBACK_KINDS.contains(&kind) || text.chars().count() < 3 {
            return Err("feedback needs a kind and at least 3 characters".into());
        }
        self.enqueue("feedback", json!({"kind": kind, "text": text, "app": VERSION})).map_err(|e| e.to_string())
    }

    pub fn pending(&self) -> Vec<Value> {
        self.read_queue()
    }

    // ------------------------------------------------------------------ sending
    /// Sends the queue in one request. Returns how many items were sent (0 if nothing or impossible).
    pub fn flush(&self, send: Send) -> usize {
        if self.api.is_empty() {
            return 0;
        }
        let items = {
            let _lock = QUEUE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            self.read_queue()
        };
        let kind = |i: &&Value, k: &str| i.get("kind").and_then(Value::as_str) == Some(k);
        let games: Vec<&Value> =
            if self.enabled() { items.iter().filter(|i| kind(i, "game")).take(MAX_GAMES).collect() } else { vec![] };
        let notes: Vec<&Value> = items.iter().filter(|i| kind(i, "feedback")).take(MAX_FEEDBACK).collect();
        if games.is_empty() && notes.is_empty() {
            return 0;
        }
        let data = |v: &Vec<&Value>| v.iter().map(|i| i.get("data").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>();
        let body = json!({"install": self.install_id(), "games": data(&games), "feedback": data(&notes)});
        let status = match send("POST", &format!("{}/v1/games", self.api), Some(&body)) {
            Ok(s) => s,
            Err(_) => return 0, // offline: try again later
        };
        // 200: stored, or refused one by one by the server (invalid). 422: all refused (old version?):
        // not sent again.
        if status != 200 && status != 422 {
            return 0;
        }
        let sent: Vec<String> = games.iter().chain(&notes).map(|i| key(i)).collect();
        let _lock = QUEUE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let left: Vec<Value> = self.read_queue().into_iter().filter(|i| !sent.contains(&key(i))).collect(); // re-read
        let _ = self.write_queue(&left);
        sent.len()
    }

    /// Deletes everything this install sent, then forgets the id and empties the queue of games.
    pub fn delete_remote(&self, send: Send) -> bool {
        let mut s = self.state();
        if let Some(id) = s.get("install").and_then(Value::as_str).filter(|i| !i.is_empty()) {
            if !self.api.is_empty() && send("DELETE", &format!("{}/v1/installs/{id}", self.api), None) != Ok(200) {
                return false;
            }
        }
        s.remove("install");
        s.insert("enabled".into(), Value::Bool(false));
        let _ = self.save(&s);
        {
            let _lock = QUEUE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let left: Vec<Value> =
                self.read_queue().into_iter().filter(|i| i.get("kind").and_then(Value::as_str) != Some("game")).collect();
            let _ = self.write_queue(&left);
        }
        let _ = std::fs::remove_file(self.last_path());
        true
    }

    /// stats.json of all players (GitHub), at most once a day.
    pub fn download_stats(&self) -> Option<Value> {
        let path = self.stats_path();
        let fresh = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() < 86400);
        if fresh {
            if let Some(v) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                return Some(v);
            }
        }
        let data = net::get_github(STATS_URL, STATS_LIMIT, 20).ok()?;
        let stats: Value = serde_json::from_slice(&data).ok()?;
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(&path, &data);
        stats.is_object().then_some(stats)
    }
}

/// Identity of a queue item (qid since 1.9; before, the content itself).
fn key(item: &Value) -> String {
    match item.get("qid").and_then(Value::as_str) {
        Some(q) if !q.is_empty() => q.to_string(),
        _ => item.to_string(), // serde_json maps keep their keys sorted
    }
}

// ------------------------------------------------------------------ game summary
fn minion(card_id: &str, a: i64, h: i64, g: bool) -> Option<Value> {
    if card_id.is_empty() {
        return None;
    }
    let id: String = card_id.chars().take(40).collect();
    Some(json!({"id": id, "a": a.clamp(0, 100_000), "h": h.clamp(0, 100_000), "g": g}))
}

fn board_of(items: impl Iterator<Item = Option<Value>>) -> Vec<Value> {
    items.flatten().take(7).collect()
}

fn snaps(board: &[crate::combat::MinionSnap]) -> Vec<Value> {
    board_of(board.iter().map(|m| minion(&m.card_id, m.attack, m.health, m.golden)))
}

/// `record` = history::game_record of the same game (hero, place, comp...).
pub fn game_summary(game: &Game, record: &Record) -> Value {
    let fights: Vec<Value> = game
        .fights
        .iter()
        .take(40)
        .filter(|f| matches!(f.result, 'W' | 'L' | 'T') && f.turn != 0)
        .map(|f| {
            let fr = f.frozen.as_ref();
            json!({
                "t": f.turn.clamp(1, 60),
                "tier": fr.map(|x| x.my_tier).unwrap_or(1).clamp(1, 7),
                "otier": fr.map(|x| x.opp_tier).unwrap_or(1).clamp(1, 7),
                "hp": f.my_hp.unwrap_or(0).clamp(-100, 200),
                "r": f.result.to_string(),
                "d": f.damage.clamp(0, 100),
                "me": fr.map(|x| snaps(&x.my_board)).unwrap_or_default(),
                "opp": f.board.as_deref().map(snaps).unwrap_or_default(),
            })
        })
        .collect();
    // To spot conceded or AFK games (server/src/validate.js, gameQuality): actions per turn (buy, sell,
    // play, roll, level, hero power... all count), health at the end and concede. The server decides.
    let turns = game.turn().clamp(1, 60);
    let actions: Vec<i64> = (1..=turns).map(|t| game.actions_by_turn.get(&t).copied().unwrap_or(0).clamp(0, 999)).collect();
    let mut races = record.races.clone();
    races.sort();
    races.truncate(6);
    let board = board_of(game.my_board().iter().map(|e| minion(&e.card_id, e.int("ATK"), e.int("HEALTH") - e.int("DAMAGE"), e.is_golden())));
    json!({
        "v": 1,
        "app": VERSION,
        "mode": if record.duos { "duos" } else { "solo" },
        "hero": record.hero,
        "deity": record.deity,
        "comp": record.comp,
        "races": races,
        "place": record.placement,
        "board": board,
        "fights": fights,
        "turns": turns,
        "actions": actions,
        "end_hp": game.my_health().unwrap_or(0).clamp(-100, 200),
        "conceded": game.conceded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn share() -> Share {
        let dir = std::env::temp_dir().join(format!("owl-share-{}", uuid::Uuid::new_v4()));
        Share { dir, api: "https://api.example.workers.dev".into() }
    }

    fn record() -> Record {
        Record {
            hero: "Arch-Villain Rafaam".into(),
            races: vec!["UNDEAD".into(), "PIRATE".into()],
            placement: 3,
            comp: Some("undead-apm".into()),
            date: "2026-11-02".into(),
            time: "12:00:41".into(),
            ..Default::default()
        }
    }

    fn ok(_: &str, _: &str, _: Option<&Value>) -> Result<u16, String> {
        Ok(200)
    }

    #[test]
    fn nothing_is_queued_until_the_player_says_yes() {
        let s = share();
        let g = Game::new();
        assert!(!s.asked() && !s.add_game(&g, &record()));
        s.set_enabled(false).unwrap();
        assert!(s.asked() && !s.add_game(&g, &record()) && s.pending().is_empty());
    }

    #[test]
    fn summary_has_only_the_listed_fields() {
        let g = Game::new();
        let v = game_summary(&g, &record());
        let mut keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["actions", "app", "board", "comp", "conceded", "deity", "end_hp", "fights", "hero", "mode", "place", "races", "turns", "v"]
        );
        assert_eq!(v["races"], json!(["PIRATE", "UNDEAD"]));
        let text = v.to_string();
        assert!(!text.contains("12:00:41") && !text.contains("2026-11-02"));
        // The server wants a plain x.y.z version.
        assert!(v["app"].as_str().unwrap().split('.').all(|p| p.parse::<u32>().is_ok()));
    }

    #[test]
    fn flush_sends_games_and_feedback_in_one_request() {
        let s = share();
        s.set_enabled(true).unwrap();
        let g = Game::new();
        assert!(s.add_game(&g, &record()) && s.add_game(&g, &record()));
        s.add_feedback("idea", "Dark mode please").unwrap();
        let calls = RefCell::new(Vec::new());
        let send = |m: &str, u: &str, b: Option<&Value>| {
            calls.borrow_mut().push((m.to_string(), u.to_string(), b.cloned()));
            Ok(200)
        };
        assert_eq!(s.flush(&send), 3);
        let calls = calls.into_inner();
        assert_eq!(calls.len(), 1);
        let body = calls[0].2.as_ref().unwrap();
        assert_eq!((body["games"].as_array().unwrap().len(), body["feedback"].as_array().unwrap().len()), (2, 1));
        assert!(s.pending().is_empty());
    }

    #[test]
    fn feedback_is_sent_without_sharing_and_kept_offline() {
        let s = share();
        s.add_feedback("bug", "Panel overlaps the leaderboard").unwrap();
        let offline = |_: &str, _: &str, _: Option<&Value>| Err("no network".to_string());
        assert_eq!(s.flush(&offline), 0);
        assert_eq!(s.pending().len(), 1);
        assert_eq!(s.flush(&ok), 1);
    }

    #[test]
    fn bad_feedback_refused() {
        let s = share();
        assert!(s.add_feedback("spam", "hello").is_err());
        assert!(s.add_feedback("idea", " ").is_err());
    }

    #[test]
    fn delete_remote_forgets_the_install_id() {
        let s = share();
        s.set_enabled(true).unwrap();
        let old = s.install_id();
        let calls = RefCell::new(Vec::new());
        let send = |m: &str, u: &str, _: Option<&Value>| {
            calls.borrow_mut().push((m.to_string(), u.to_string()));
            Ok(200)
        };
        assert!(s.delete_remote(&send));
        assert_eq!(calls.into_inner(), [("DELETE".to_string(), format!("https://api.example.workers.dev/v1/installs/{old}"))]);
        assert!(!s.enabled() && !s.state().contains_key("install"));
    }

    #[test]
    fn feedback_typed_during_a_send_is_kept() {
        let s = share();
        s.add_feedback("idea", "first idea").unwrap();
        let slow = |_: &str, _: &str, _: Option<&Value>| {
            s.add_feedback("bug", "typed while sending").unwrap();
            Ok(200)
        };
        assert_eq!(s.flush(&slow), 1);
        let left: Vec<String> = s.pending().iter().map(|i| i["data"]["text"].as_str().unwrap().to_string()).collect();
        assert_eq!(left, ["typed while sending"]);
    }

    #[test]
    fn python_queue_items_are_read() {
        let s = share();
        std::fs::create_dir_all(&s.dir).unwrap();
        let line = format!("{{\"kind\": \"feedback\", \"data\": {{\"kind\": \"idea\", \"text\": \"old one\", \"app\": \"1.9.0\"}}, \"queued\": {}, \"qid\": \"abc\"}}\n", now());
        std::fs::write(s.dir.join("share_queue.jsonl"), line).unwrap();
        assert_eq!(s.flush(&ok), 1);
    }
}
