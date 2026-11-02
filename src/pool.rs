//! Minions by tier, filtered by the game's tribes (like HDT's minion browser).
//!
//! - Sections of a tier: "ALL" (Amalgams) first, then one per tribe, then "NEUTRAL" (Other), then minions
//!   whose tier changes each game, then Tavern spells sorted by cost.
//! - A dual-tribe minion appears in the section of each of its tribes in the game.
//! - A neutral "tied" to a tribe only appears if that tribe is in the game. The game says so with
//!   BACON_SUBSET_<TRIBE> tags, missing from HearthstoneJSON: they are LEARNED from the logs
//!   (subsets.json, in the user folder). Fallback (HDT's rule): a neutral whose text names ONE tribe is
//!   filed under that tribe.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock};

use regex::Regex;

use crate::cards::{Card, CardDb};
use crate::game::Game;
use crate::paths;

/// Subsets: card id -> tribes it is tied to.
pub type Subsets = BTreeMap<String, Vec<String>>;

/// Log tag -> tribe (names differ: MECH/MECHANICAL, QUILLBOAR/QUILBOAR).
pub const SUBSET_TAG_RACE: [(&str, &str); 11] = [
    ("BACON_SUBSET_BEAST", "BEAST"),
    ("BACON_SUBSET_DEMON", "DEMON"),
    ("BACON_SUBSET_DRAGON", "DRAGON"),
    ("BACON_SUBSET_ELEMENTALS", "ELEMENTAL"),
    ("BACON_SUBSET_MECH", "MECHANICAL"),
    ("BACON_SUBSET_MURLOC", "MURLOC"),
    ("BACON_SUBSET_NAGA", "NAGA"),
    ("BACON_SUBSET_PIRATE", "PIRATE"),
    ("BACON_SUBSET_QUILLBOAR", "QUILBOAR"),
    ("BACON_SUBSET_UNDEAD", "UNDEAD"),
    ("BACON_SUBSET_ABERRATION", "ABERRATION"),
];

/// Display name of each tribe / section (and the word looked for in card texts).
pub fn race_label(key: &str) -> &str {
    match key {
        "BEAST" => "Beast",
        "DEMON" => "Demon",
        "DRAGON" => "Dragon",
        "ELEMENTAL" => "Elemental",
        "MECHANICAL" => "Mech",
        "MURLOC" => "Murloc",
        "NAGA" => "Naga",
        "PIRATE" => "Pirate",
        "QUILBOAR" => "Quilboar",
        "UNDEAD" => "Undead",
        "ABERRATION" => "Aberration",
        "ALL" => "All types",
        "NEUTRAL" => "Other",
        "SPELL" => "Spells",
        "VARIES" => "Tier changes each game",
        other => other,
    }
}

const TRIBES: [&str; 11] =
    ["BEAST", "DEMON", "DRAGON", "ELEMENTAL", "MECHANICAL", "MURLOC", "NAGA", "PIRATE", "QUILBOAR", "UNDEAD", "ABERRATION"];

/// Minions whose tier changes each game (the card file gives a fake tier): listed apart, at the bottom
/// of each tier.
pub const VARIABLE_TIER: [&str; 1] = ["Dark Paradox"];

/// Keyword filter (like HDT): label -> game tag the card has or mentions.
pub const KEYWORDS: [(&str, &str); 13] = [
    ("Battlecry", "BATTLECRY"),
    ("Deathrattle", "DEATHRATTLE"),
    ("Rally", "BACON_RALLY"),
    ("End of Turn", "END_OF_TURN_TRIGGER"),
    ("Start of Combat", "START_OF_COMBAT"),
    ("Divine Shield", "DIVINE_SHIELD"),
    ("Taunt", "TAUNT"),
    ("Reborn", "REBORN"),
    ("Magnetic", "MAGNETIC"),
    ("Avenge", "AVENGE"),
    ("Choose One", "CHOOSE_ONE"),
    ("Windfury", "WINDFURY"),
    ("Venomous", "VENOMOUS"),
];

// ------------------------------------------------------------------- learning subsets
fn subsets_path() -> std::path::PathBuf {
    paths::user_dir().join("subsets.json")
}

/// What was learned in my games (or the copy in a source checkout's data folder).
pub fn load_subsets() -> Subsets {
    let mut candidates = vec![subsets_path()];
    if let Some(d) = paths::data_dir() {
        candidates.push(d.join("subsets.json"));
    }
    for path in candidates {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(s) = serde_json::from_str(&text) {
                return s;
            }
        }
    }
    Subsets::new()
}

/// `load_subsets()`, read again at most every 10 seconds (the window asks several times a second).
pub fn load_subsets_cached() -> Arc<Subsets> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, Arc<Subsets>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, s)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(10) {
            return s.clone();
        }
    }
    let s = Arc::new(load_subsets());
    *cache = Some((Instant::now(), s.clone()));
    s
}

/// Saves in the user folder (never in the data folder).
pub fn save_subsets(subsets: &Subsets) -> std::io::Result<()> {
    let path = subsets_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(subsets).unwrap_or_default())?;
    std::fs::rename(tmp, path)
}

/// Adds the BACON_SUBSET tags seen on this game's pool minions.
pub fn learn_subsets(game: &Game, db: &CardDb, subsets: &mut Subsets) {
    for e in game.entities() {
        let Some(card) = (if e.card_id.is_empty() { None } else { db.base(&e.card_id) }) else { continue };
        if !card.is_pool_minion {
            continue;
        }
        let mut races: Vec<String> =
            SUBSET_TAG_RACE.iter().filter(|(tag, _)| e.truthy(tag)).map(|(_, r)| r.to_string()).collect();
        if races.is_empty() {
            continue;
        }
        let known = subsets.entry(card.id.clone()).or_default();
        races.append(known);
        races.sort();
        races.dedup();
        *known = races;
    }
}

// ------------------------------------------------------------------- sorting cards
static RACE_WORD: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    TRIBES
        .iter()
        .map(|r| (*r, Regex::new(&format!(r"\b{}s?\b", race_label(r))).expect("valid regex")))
        .collect()
});

/// Tribes named in the text (HDT's fallback rule for neutrals).
fn races_in_text(card: &Card) -> Vec<String> {
    RACE_WORD.iter().filter(|(_, re)| re.is_match(&card.text)).map(|(r, _)| r.to_string()).collect()
}

/// Sections to file the card in: its tribes, else its subset, else "NEUTRAL".
pub fn card_groups(card: &Card, subsets: &Subsets) -> Vec<String> {
    if card.has_race("ALL") {
        return vec!["ALL".into()];
    }
    if !card.races.is_empty() {
        return card.races.clone();
    }
    if let Some(s) = subsets.get(&card.id) {
        return s.clone();
    }
    let found = races_in_text(card);
    if found.len() == 1 { found } else { vec!["NEUTRAL".into()] }
}

/// Can the card show up in this game? races=None: not known yet -> yes.
pub fn is_available(groups: &[String], races: Option<&[String]>) -> bool {
    let Some(races) = races else { return true };
    if groups == ["ALL"] || groups == ["NEUTRAL"] {
        return true;
    }
    groups.iter().any(|g| races.contains(g))
}

/// Keyword labels that at least one minion possible in this game has.
pub fn available_keywords(db: &CardDb, races: Option<&[String]>, subsets: &Subsets, duos: bool) -> Vec<&'static str> {
    let tags: std::collections::HashSet<String> = db
        .pool_minions(duos)
        .iter()
        .filter(|c| is_available(&card_groups(c, subsets), races))
        .flat_map(|c| c.keywords.clone())
        .collect();
    KEYWORDS.iter().filter(|(_, tag)| tags.contains(*tag)).map(|(label, _)| *label).collect()
}

/// Is tier 7 worth showing? Yes before the tribes are known, if a tier 7 card was seen already (shop,
/// board, hand: heroes, trinkets or quests give them), or if a minion possible in this lobby discovers a
/// tier 7 (Greedy Conniver, so Pirates).
pub fn tier7_reachable(db: &CardDb, races: Option<&[String]>, subsets: &Subsets, duos: bool, seen_tier7: bool) -> bool {
    if races.is_none() || seen_tier7 {
        return true;
    }
    db.pool_minions(duos).iter().any(|c| c.text.contains("Tier 7") && is_available(&card_groups(c, subsets), races))
}

/// Sections of the tier: (section code, cards sorted by name).
///
/// races: sure tribes of the game (None = all, before detection); keyword: a KEYWORDS label to keep
/// only the minions that have it (spells are then hidden).
pub fn tier_list(
    db: &CardDb,
    tier: i64,
    races: Option<&[String]>,
    subsets: &Subsets,
    duos: bool,
    keyword: Option<&str>,
) -> Vec<(String, Vec<Arc<Card>>)> {
    let tag = keyword.and_then(|k| KEYWORDS.iter().find(|(l, _)| *l == k)).map(|(_, t)| *t);
    let mut sections: HashMap<String, Vec<Arc<Card>>> = HashMap::new();
    for card in db.pool_minions(duos) {
        if let Some(tag) = tag {
            if !card.has_keyword(tag) {
                continue;
            }
        }
        if VARIABLE_TIER.contains(&card.name.as_str()) {
            sections.entry("VARIES".into()).or_default().push(card);
            continue;
        }
        if card.tier != Some(tier) {
            continue;
        }
        let groups = card_groups(&card, subsets);
        if !is_available(&groups, races) {
            continue;
        }
        for g in groups {
            if g == "ALL" || g == "NEUTRAL" || races.is_none_or(|r| r.contains(&g)) {
                sections.entry(g).or_default().push(card.clone());
            }
        }
    }
    // Same order as HDT: ALL, tribes (alphabetical label), Other, "tier changes", Spells.
    let order = |key: &str| {
        let rank = match key {
            "ALL" => 0,
            "NEUTRAL" => 2,
            "VARIES" => 3,
            _ => 1,
        };
        (rank, race_label(key).to_string())
    };
    let mut result: Vec<(String, Vec<Arc<Card>>)> = sections
        .into_iter()
        .map(|(k, mut v)| {
            v.sort_by(|a, b| a.name.cmp(&b.name));
            (k, v)
        })
        .collect();
    result.sort_by_key(|(k, _)| order(k));
    if tag.is_none() {
        let mut spells: Vec<Arc<Card>> = db.pool_spells(duos).into_iter().filter(|c| c.tier == Some(tier)).collect();
        spells.sort_by(|a, b| (a.cost, &a.name).cmp(&(b.cost, &b.name)));
        if !spells.is_empty() {
            result.push(("SPELL".into(), spells));
        }
    }
    result
}

/// Minions of this lobby from tier 1 to `tier` (for the chances of finding a card when rolling).
pub fn pool_up_to(db: &CardDb, tier: i64, races: Option<&[String]>, subsets: &Subsets, duos: bool) -> Vec<Arc<Card>> {
    let mut found: BTreeMap<String, Arc<Card>> = BTreeMap::new();
    for t in 1..=tier {
        for (section, cards) in tier_list(db, t, races, subsets, duos, None) {
            if section != "SPELL" && section != "VARIES" {
                for c in cards {
                    found.insert(c.id.clone(), c);
                }
            }
        }
    }
    found.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn dual_race_rules() {
        assert!(is_available(&s(&["DEMON", "DRAGON"]), Some(&s(&["DRAGON", "BEAST"]))));
        assert!(!is_available(&s(&["DEMON", "PIRATE"]), Some(&s(&["DRAGON", "BEAST"]))));
        assert!(is_available(&s(&["NEUTRAL"]), Some(&s(&["DRAGON"]))));
        assert!(is_available(&s(&["UNDEAD"]), None));
    }

    #[test]
    fn subset_neutral_grouped_by_race() {
        let db = cards::test_db();
        let pool = db.pool_minions(false);
        let g = pool.iter().find(|c| c.name == "Disguised Graverobber").unwrap();
        let mut subsets = Subsets::new();
        subsets.insert(g.id.clone(), s(&["UNDEAD"]));
        assert_eq!(card_groups(g, &subsets), s(&["UNDEAD"]));
    }

    #[test]
    fn keyword_filter_and_dark_paradox_apart() {
        let db = cards::test_db();
        let lobby = s(&["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"]);
        let none = Subsets::new();
        assert!(available_keywords(&db, Some(&lobby), &none, false).contains(&"Deathrattle"));
        let sections = tier_list(&db, 5, Some(&lobby), &none, false, Some("Deathrattle"));
        let names: Vec<&str> = sections.iter().flat_map(|(_, c)| c.iter().map(|c| c.name.as_str())).collect();
        assert!(names.contains(&"Ghastcoiler") && names.contains(&"Titus Rivendare"));
        assert!(sections.iter().all(|(k, _)| k != "SPELL"));
        for tier in [1, 4] {
            let sections = tier_list(&db, tier, Some(&lobby), &none, false, None);
            let varies = &sections.iter().find(|(k, _)| k == "VARIES").unwrap().1;
            assert_eq!(varies.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Dark Paradox"]);
            assert_eq!(sections.last().unwrap().0, "SPELL");
        }
    }

    #[test]
    fn tier7_reachable_only_with_a_way_in() {
        let db = cards::test_db();
        let none = Subsets::new();
        let lobby = s(&["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"]);
        assert!(!tier7_reachable(&db, Some(&lobby), &none, false, false));
        let pirates = s(&["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "PIRATE"]);
        assert!(tier7_reachable(&db, Some(&pirates), &none, false, false));
        assert!(tier7_reachable(&db, Some(&lobby), &none, false, true));
        assert!(tier7_reachable(&db, None, &none, false, false));
    }
}
