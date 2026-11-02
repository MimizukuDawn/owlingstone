//! Card database: name, tier and tribes of every card, from HearthstoneJSON.
//!
//! Who does what:
//! - HearthstoneJSON publishes every card of the game in one free JSON file.
//! - `refresh_in_background` downloads it to `cards.json` in the user folder; `db()` returns the
//!   database loaded in memory (an `Arc`, swapped in one go when a fresh one is ready).
//! - `data/cards-fallback.json.gz` is a reduced copy built into the exe: a first start without internet
//!   (or with HearthstoneJSON down) works with it instead of failing.
//! - Background refresh, never on the UI thread: at start if `cards.json` is missing or older than
//!   7 days, and when the game shows a card the database does not know (new patch).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime};

use arc_swap::ArcSwapOption;
use regex::Regex;
use serde::Deserialize;

use crate::{data, net, paths, season};

pub const CARDS_URL: &str = "https://api.hearthstonejson.com/v1/latest/enUS/cards.json";
const MAX_AGE: Duration = Duration::from_secs(7 * 86400);
/// After an unknown card, at most one download every 6 hours.
const UNKNOWN_RETRY: Duration = Duration::from_secs(6 * 3600);
/// Tests and offline runs: never touch the network.
pub const NO_DOWNLOAD: &str = "OWLINGSTONE_NO_CARD_DOWNLOAD";
const MAX_DOWNLOAD: u64 = 100_000_000;

/// One card, in the shape the rest of the app needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// Technical id, e.g. "BG20_100".
    pub id: String,
    /// Numeric id (links golden and normal versions).
    pub dbf_id: i64,
    /// Display name, e.g. "Razorfen Geomancer".
    pub name: String,
    /// Tavern tier 1 to 7 (None if not a Battlegrounds minion or spell).
    pub tier: Option<i64>,
    /// e.g. ["QUILBOAR"]; ["ALL"] for Amalgams.
    pub races: Vec<String>,
    /// True if Bob can offer it in the tavern.
    pub is_pool_minion: bool,
    /// True if it is a playable Battlegrounds hero.
    pub is_hero: bool,
    /// Card text (HTML-ish, as published).
    pub text: String,
    pub attack: i64,
    pub health: i64,
    pub cost: i64,
    /// A Tavern spell Bob sells.
    pub is_pool_spell: bool,
    /// Exists only in Duos (never offered in Solo).
    pub is_duos_only: bool,
    /// Game keywords it has or mentions (DEATHRATTLE, BACON_RALLY...).
    pub keywords: Vec<String>,
}

impl Card {
    pub fn has_keyword(&self, tag: &str) -> bool {
        self.keywords.iter().any(|k| k == tag)
    }
    pub fn has_race(&self, race: &str) -> bool {
        self.races.iter().any(|r| r == race)
    }
}

#[derive(Deserialize)]
struct RawCard {
    id: String,
    #[serde(rename = "dbfId", default)]
    dbf_id: i64,
    name: Option<String>,
    #[serde(rename = "techLevel")]
    tech_level: Option<i64>,
    races: Option<Vec<String>>,
    race: Option<String>,
    #[serde(rename = "isBattlegroundsPoolMinion", default)]
    pool_minion: bool,
    #[serde(rename = "battlegroundsHero", default)]
    hero: bool,
    #[serde(default)]
    text: String,
    #[serde(default)]
    attack: i64,
    #[serde(default)]
    health: i64,
    #[serde(default)]
    cost: i64,
    #[serde(rename = "isBattlegroundsPoolSpell", default)]
    pool_spell: bool,
    #[serde(rename = "isBattlegroundsDuosExclusive", default)]
    duos_only: bool,
    #[serde(default)]
    mechanics: Vec<String>,
    #[serde(rename = "referencedTags", default)]
    referenced: Vec<String>,
    #[serde(rename = "battlegroundsNormalDbfId")]
    normal_dbf: Option<i64>,
}

pub struct CardDb {
    pub by_id: HashMap<String, Arc<Card>>,
    pub by_dbf: HashMap<i64, Arc<Card>>,
    /// Golden version id -> normal version id.
    normal_of: HashMap<String, String>,
    /// Where it was read from (for the logs).
    pub source: String,
}

impl CardDb {
    /// Builds the database from the JSON text of HearthstoneJSON's card list.
    pub fn from_json(text: &[u8], source: &str) -> Result<CardDb, String> {
        let raw: Vec<RawCard> = serde_json::from_slice(text).map_err(|e| format!("cards: {e}"))?;
        if raw.len() < 1000 {
            return Err("cards: not the card list".into());
        }
        let mut by_id = HashMap::with_capacity(raw.len());
        let mut by_dbf = HashMap::with_capacity(raw.len());
        let mut normal_dbf = Vec::new();
        for c in raw {
            let races = match (c.races, c.race) {
                (Some(r), _) if !r.is_empty() => r,
                (_, Some(r)) => vec![r],
                _ => vec![],
            };
            let mut keywords = c.mechanics;
            keywords.extend(c.referenced);
            let card = Arc::new(Card {
                name: c.name.unwrap_or_else(|| c.id.clone()),
                id: c.id,
                dbf_id: c.dbf_id,
                tier: c.tech_level,
                races,
                is_pool_minion: c.pool_minion,
                is_hero: c.hero,
                text: c.text,
                attack: c.attack,
                health: c.health,
                cost: c.cost,
                is_pool_spell: c.pool_spell,
                is_duos_only: c.duos_only,
                keywords,
            });
            if let Some(n) = c.normal_dbf {
                normal_dbf.push((card.id.clone(), n));
            }
            by_dbf.insert(card.dbf_id, card.clone());
            by_id.insert(card.id.clone(), card);
        }
        // "battlegroundsNormalDbfId" points from the golden version to the normal one.
        let normal_of = normal_dbf
            .into_iter()
            .filter_map(|(id, n)| by_dbf.get(&n).map(|c| (id, c.id.clone())))
            .collect();
        Ok(CardDb { by_id, by_dbf, normal_of, source: source.to_string() })
    }

    /// The card with this id. An unknown id that looks like a real card id starts a refresh.
    pub fn get(&self, card_id: &str) -> Option<&Arc<Card>> {
        let card = self.by_id.get(card_id);
        if card.is_none() && !card_id.is_empty() {
            unknown(card_id);
        }
        card
    }

    /// The normal (not golden) card for this id.
    pub fn base(&self, card_id: &str) -> Option<&Arc<Card>> {
        let id = self.normal_of.get(card_id).map(String::as_str).unwrap_or(card_id);
        let card = self.by_id.get(id);
        if card.is_none() && !card_id.is_empty() {
            unknown(card_id);
        }
        card
    }

    /// Normal card id, or the id itself when unknown.
    pub fn base_id<'a>(&'a self, card_id: &'a str) -> &'a str {
        self.by_id
            .get(self.normal_of.get(card_id).map(String::as_str).unwrap_or(card_id))
            .map(|c| c.id.as_str())
            .unwrap_or(card_id)
    }

    /// Minions Bob can offer: no Duos-only cards in Solo, no tribes rotated out of the pool.
    pub fn pool_minions(&self, duos: bool) -> Vec<Arc<Card>> {
        let rotated = season::get().rotated_out.clone();
        let mut out: Vec<Arc<Card>> = self
            .by_id
            .values()
            .filter(|c| c.is_pool_minion && in_mode(c, duos))
            .filter(|c| c.races.is_empty() || !c.races.iter().all(|r| rotated.contains(r)))
            .cloned()
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn pool_spells(&self, duos: bool) -> Vec<Arc<Card>> {
        let mut out: Vec<Arc<Card>> =
            self.by_id.values().filter(|c| c.is_pool_spell && in_mode(c, duos)).cloned().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }
}

/// A Duos card does not exist in Solo; in Duos the whole pool is possible.
fn in_mode(card: &Card, duos: bool) -> bool {
    duos || !card.is_duos_only
}

// ------------------------------------------------------------------ loading and refreshing
static DB: ArcSwapOption<CardDb> = ArcSwapOption::const_empty();
static LOAD_LOCK: Mutex<()> = Mutex::new(());
/// Increases each time a new database is swapped in (data that depends on it is rebuilt).
static GENERATION: AtomicU64 = AtomicU64::new(0);
static REFRESHING: AtomicBool = AtomicBool::new(false);
static LAST_TRY: Mutex<Option<SystemTime>> = Mutex::new(None);
static CARD_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9]+_[A-Za-z0-9_]+$").unwrap());

pub fn cards_path() -> PathBuf {
    paths::user_dir().join("cards.json")
}

fn read_file(path: &Path) -> Result<CardDb, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    CardDb::from_json(&bytes, &path.display().to_string())
}

/// The database built into the exe (data/cards-fallback.json.gz).
pub fn fallback() -> CardDb {
    let mut text = Vec::new();
    flate2::read::GzDecoder::new(data::CARDS_FALLBACK)
        .read_to_end(&mut text)
        .expect("built-in card data is a valid gzip file");
    CardDb::from_json(&text, "built-in").expect("built-in card data is valid")
}

/// The card database, loaded once: the user's cards.json, else the built-in copy.
/// Never downloads on the calling thread (the UI never freezes, and nothing fails offline).
pub fn db() -> Arc<CardDb> {
    if let Some(db) = DB.load_full() {
        return db;
    }
    let _guard = LOAD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(db) = DB.load_full() {
        return db;
    }
    let db = Arc::new(read_file(&cards_path()).unwrap_or_else(|_| fallback()));
    DB.store(Some(db.clone()));
    GENERATION.fetch_add(1, Ordering::SeqCst);
    if stale() {
        refresh_in_background(false);
    }
    db
}

/// Number of database loads so far (changes when a fresh card list arrives).
pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

/// Uses this database from now on (tests, or a fresh download).
pub fn install(db: CardDb) {
    DB.store(Some(Arc::new(db)));
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

fn stale() -> bool {
    match std::fs::metadata(cards_path()).and_then(|m| m.modified()) {
        Ok(t) => SystemTime::now().duration_since(t).unwrap_or_default() > MAX_AGE,
        Err(_) => true,
    }
}

/// Downloads cards.json and checks it is the full card list before replacing the old one.
pub fn download(path: &Path) -> Result<(), String> {
    let data = net::get(CARDS_URL, MAX_DOWNLOAD, 60)?;
    let db = CardDb::from_json(&data, &path.display().to_string())?;
    drop(db);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, &data).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Downloads cards.json on its own thread, at most once every UNKNOWN_RETRY unless `force`.
/// Returns true if a download started.
pub fn refresh_in_background(force: bool) -> bool {
    if std::env::var_os(NO_DOWNLOAD).is_some() {
        return false;
    }
    {
        let mut last = LAST_TRY.lock().unwrap_or_else(|e| e.into_inner());
        let now = SystemTime::now();
        if !force {
            if let Some(t) = *last {
                if now.duration_since(t).unwrap_or_default() < UNKNOWN_RETRY {
                    return false;
                }
            }
        }
        if REFRESHING.swap(true, Ordering::SeqCst) {
            return false;
        }
        *last = Some(now);
    }
    let started = std::thread::Builder::new().name("cards-refresh".into()).spawn(|| {
        let path = cards_path();
        match download(&path).and_then(|_| read_file(&path)) {
            Ok(fresh) => install(fresh),
            Err(e) => crate::errlog::note("card data refresh", &e),
        }
        REFRESHING.store(false, Ordering::SeqCst);
    });
    if started.is_err() {
        REFRESHING.store(false, Ordering::SeqCst);
    }
    started.is_ok()
}

/// The game shows a card the database does not have: probably a new patch. Download again, unless
/// cards.json was just downloaded (some ids in the log exist nowhere).
fn unknown(card_id: &str) {
    if !CARD_ID.is_match(card_id) {
        return;
    }
    let fresh = std::fs::metadata(cards_path())
        .and_then(|m| m.modified())
        .map(|t| SystemTime::now().duration_since(t).unwrap_or_default() < UNKNOWN_RETRY)
        .unwrap_or(false);
    if !fresh {
        refresh_in_background(false);
    }
}

/// Card text without its markup (`<b>`, `[x]`...), on one line: for tooltips and text matching.
pub fn plain_text(text: &str) -> String {
    static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>|\[x\]").unwrap());
    TAGS.replace_all(text, "").replace('\n', " ").trim().to_string()
}

#[cfg(test)]
pub(crate) fn test_db() -> Arc<CardDb> {
    // SAFETY of the test setup: every test uses the built-in data and never downloads.
    // SAFETY: tests set this before any thread reads the environment.
    unsafe { std::env::set_var(NO_DOWNLOAD, "1") };
    db()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_has_pool_and_golden_links() {
        let db = test_db();
        let vermin = db.get("BG31_803").unwrap();
        assert_eq!(vermin.name, "Buzzing Vermin");
        assert_eq!(vermin.tier, Some(1));
        assert!(vermin.has_keyword("DEATHRATTLE"));
        assert!(db.pool_minions(false).len() > 100);
        assert!(db.pool_spells(false).len() > 10);
        // golden ids lead to the normal card
        let golden = db.by_id.values().find(|c| db.normal_of.contains_key(&c.id)).unwrap();
        assert_ne!(db.base(&golden.id).unwrap().id, golden.id);
        // rotated tribes (Naga) are out of the Solo pool
        assert!(db.pool_minions(false).iter().all(|c| !(c.races == ["NAGA"])));
    }

    #[test]
    fn plain_text_strips_markup() {
        assert_eq!(plain_text("[x]<b>Taunt</b>\n<b>Deathrattle:</b> Summon a\n2/2 Beetle."),
                   "Taunt Deathrattle: Summon a 2/2 Beetle.");
    }
}
