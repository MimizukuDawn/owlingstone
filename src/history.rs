//! My game history: one line per finished game (history.jsonl in the user folder), and my own stats.
//!
//! - `game_record(game)` sums up a finished game: date, hero, Deity, tribes, final board, comp
//!   recognised on that board, final place.
//! - `add_records(...)` appends games to history.jsonl without duplicates: a game is known by its date,
//!   start time and my hero.
//! - `import_logs(paths)` reads old Power.log files to fill the history at once.
//! - `stats(records, key)`: my average place by comp, by Deity, by hero...
//! - `rule_report(records, rules)`: for each rule of rules.yaml, my average with its comps when the
//!   condition was true, and when it was false. That's how a rule like "Aberration Deathrattle if the
//!   Deity is C'Thun" is checked.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::cards;
use crate::comps;
use crate::game::Game;
use crate::knowledge::{Knowledge, Rule};
use crate::paths;
use crate::powerlog::PowerLogParser;
use crate::rules::{self, Ctx};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub date: String,
    pub time: String,
    pub hero: String,
    #[serde(default)]
    pub deity: Option<String>,
    #[serde(default)]
    pub races: Vec<String>,
    #[serde(default)]
    pub duos: bool,
    pub placement: i64,
    #[serde(default)]
    pub board: Vec<String>,
    #[serde(default)]
    pub comp: Option<String>,
    #[serde(default)]
    pub comp_confidence: i64,
}

impl Record {
    fn key(&self) -> (String, String, String) {
        (self.date.clone(), self.time.clone(), self.hero.clone())
    }
}

pub fn history_path() -> PathBuf {
    paths::user_dir().join("history.jsonl")
}

/// The game logs folder "Hearthstone_2026_10_26_11_58_02" gives the session's date.
static FOLDER_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Hearthstone_(\d{4})_(\d{2})_(\d{2})").expect("valid regex"));

/// Date of the game session from the log's folder (today if not found).
pub fn log_date(path: Option<&Path>) -> String {
    path.and_then(|p| FOLDER_DATE.captures(&p.to_string_lossy()).map(|m| format!("{}-{}-{}", &m[1], &m[2], &m[3])))
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string())
}

/// A finished Battlegrounds game (None if not finished or not readable).
pub fn game_record(game: &Game, k: &Knowledge, day: &str) -> Option<Record> {
    let placement = game.my_placement()?;
    let hero = game.my_hero()?;
    if !game.is_battlegrounds() || !game.is_over() {
        return None;
    }
    let db = cards::db();
    let board: Vec<_> = game.my_board().iter().filter(|e| !e.card_id.is_empty()).filter_map(|e| db.base(&e.card_id).cloned()).collect();
    let mut races = game.available_races();
    let ctx = Ctx { races: if game.races_complete() { Some(races.clone()) } else { None }, ..Default::default() };
    let best = comps::match_comps(&k.comps, &board, &[], &[], &ctx, 1, 0);
    races.sort();
    Some(Record {
        date: day.to_string(),
        time: game.start_time.clone(),
        hero: hero.name(),
        deity: game.deity(),
        races,
        duos: game.is_duos(),
        placement,
        board: board.iter().map(|c| c.name.clone()).collect(),
        comp: best.first().map(|m| m.comp.id.clone()),
        comp_confidence: best.first().map(|m| m.confidence).unwrap_or(0),
    })
}

pub fn load_history(path: &Path) -> Vec<Record> {
    std::fs::read_to_string(path)
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect())
        .unwrap_or_default()
}

/// Appends the games not known yet. Returns how many were added.
pub fn add_records(records: &[Record], path: &Path) -> std::io::Result<usize> {
    let mut known: HashSet<_> = load_history(path).iter().map(Record::key).collect();
    let new: Vec<&Record> = records.iter().filter(|r| known.insert(r.key())).collect();
    if !new.is_empty() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        for r in &new {
            writeln!(f, "{}", serde_json::to_string(r).unwrap_or_default())?;
        }
    }
    Ok(new.len())
}

/// Every finished Battlegrounds game of these Power.log files.
pub fn import_logs(paths: &[PathBuf], k: &Knowledge) -> Vec<Record> {
    let mut records = Vec::new();
    for path in paths {
        let mut parser = PowerLogParser::new(true);
        if parser.feed_file(path).is_err() {
            continue;
        }
        let day = log_date(Some(path));
        records.extend(parser.games().iter().filter_map(|g| game_record(g, k, &day)));
    }
    records
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub games: usize,
    pub avg: f64,
    /// % of games in the top 4.
    pub top4: i64,
}

fn summary(group: &[&Record]) -> Summary {
    let n = group.len().max(1) as f64;
    let avg = group.iter().map(|r| r.placement as f64).sum::<f64>() / n;
    let top4 = group.iter().filter(|r| r.placement <= 4).count() as f64;
    Summary { games: group.len(), avg: (avg * 100.0).round() / 100.0, top4: (100.0 * top4 / n).round_ties_even() as i64 }
}

/// [(value, summary)] grouped by key ("comp", "deity", "hero"), best first.
pub fn stats(records: &[Record], key: &str) -> Vec<(String, Summary)> {
    let mut groups: BTreeMap<String, Vec<&Record>> = BTreeMap::new();
    for r in records {
        let value = match key {
            "comp" => r.comp.clone(),
            "deity" => r.deity.clone(),
            "hero" => Some(r.hero.clone()),
            _ => None,
        };
        groups.entry(value.filter(|v| !v.is_empty()).unwrap_or_else(|| "?".into())).or_default().push(r);
    }
    let mut out: Vec<(String, Summary)> = groups.into_iter().map(|(k, g)| (k, summary(&g))).collect();
    out.sort_by(|a, b| a.1.avg.partial_cmp(&b.1.avg).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// For each rule: my average with its comps when its condition was true / false.
pub fn rule_report<'r>(records: &[Record], rules: &'r [Rule]) -> Vec<(&'r Rule, Option<Summary>, Option<Summary>)> {
    rules
        .iter()
        .map(|rule| {
            let (mut yes, mut no) = (Vec::new(), Vec::new());
            for r in records.iter().filter(|r| r.comp.as_ref().is_some_and(|c| rule.comps.contains(c))) {
                let ctx = Ctx { races: Some(r.races.clone()), deity: r.deity.clone(), hero: Some(r.hero.clone()), hero_power: None };
                if rules::check(&rule.cond, &ctx) == Some(true) { yes.push(r) } else { no.push(r) }
            }
            let s = |g: &Vec<&Record>| if g.is_empty() { None } else { Some(summary(g)) };
            (rule, s(&yes), s(&no))
        })
        .collect()
}

/// Readable text for the console: overall averages, by comp, Deity, hero, then the rules.
pub fn format_report(records: &[Record], rules: &[Rule]) -> String {
    if records.is_empty() {
        return "No game in the history yet. Run: Owlingstone history --import".into();
    }
    let all: Vec<&Record> = records.iter().collect();
    let s = summary(&all);
    let mut lines = vec![format!("Games: {} · average place {:.2} · top 4: {}%", s.games, s.avg, s.top4)];
    for (key, title) in [("comp", "By comp"), ("deity", "By Deity"), ("hero", "By hero")] {
        lines.push(String::new());
        lines.push(format!("{title} (average place, games):"));
        for (k, s) in stats(records, key) {
            lines.push(format!("  {k:<32} {:.2}  ({})", s.avg, s.games));
        }
    }
    if !rules.is_empty() {
        let fmt = |s: Option<Summary>| s.map(|s| format!("{:.2} ({})", s.avg, s.games)).unwrap_or_else(|| "–".into());
        lines.push(String::new());
        lines.push("My rules (average place with these comps: condition true / false):".into());
        for (rule, yes, no) in rule_report(records, rules) {
            lines.push(format!("  {:?} {:?} → {}: {} / {}", rule.effect, rule.cond, rule.comps.join(", "), fmt(yes), fmt(no)));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{Cond, Effect};

    fn rec(placement: i64, comp: &str, deity: &str, time: &str) -> Record {
        Record {
            date: "2026-10-26".into(),
            time: time.into(),
            hero: "Thorim, Stormlord".into(),
            deity: Some(deity.into()),
            placement,
            races: ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"].iter().map(|s| s.to_string()).collect(),
            comp: Some(comp.into()),
            ..Default::default()
        }
    }

    #[test]
    fn log_date_from_folder() {
        assert_eq!(log_date(Some(Path::new(r"C:\Logs\Hearthstone_2026_10_26_11_58_02\Power.log"))), "2026-10-26");
    }

    #[test]
    fn add_records_skips_duplicates() {
        let dir = std::env::temp_dir().join(format!("owl-hist-{}", uuid::Uuid::new_v4()));
        let path = dir.join("history.jsonl");
        assert_eq!(add_records(&[rec(1, "a", "C'Thun", "12:00:00"), rec(2, "b", "C'Thun", "13:00:00")], &path).unwrap(), 2);
        assert_eq!(add_records(&[rec(1, "a", "C'Thun", "12:00:00"), rec(5, "c", "C'Thun", "14:00:00")], &path).unwrap(), 1);
        assert_eq!(load_history(&path).iter().map(|r| r.placement).collect::<Vec<_>>(), [1, 2, 5]);
        // Python-written lines (spaces, missing fields) still read.
        std::fs::write(&path, "{\"date\": \"2026-10-01\", \"time\": \"10:00:00\", \"hero\": \"X\", \"placement\": 3}\n").unwrap();
        assert_eq!(load_history(&path)[0].placement, 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stats_and_rule_report() {
        let records = vec![
            rec(1, "aberration-deathrattle", "C'Thun", "1"),
            rec(3, "aberration-deathrattle", "C'Thun", "2"),
            rec(7, "aberration-deathrattle", "Y'Shaarj", "3"),
            rec(4, "mech-apm", "C'Thun", "4"),
        ];
        let by_comp = stats(&records, "comp");
        let ad = by_comp.iter().find(|(k, _)| k == "aberration-deathrattle").unwrap().1;
        assert_eq!(ad, Summary { games: 3, avg: 3.67, top4: 67 });
        assert_eq!(stats(&records, "deity")[0].0, "C'Thun");
        let rule = Rule {
            comps: vec!["aberration-deathrattle".into()],
            effect: Effect::PreferIf,
            cond: Cond { deity: Some("C'Thun".into()), ..Default::default() },
            bonus: 3.0,
            note: String::new(),
        };
        let rules = [rule];
        let report = rule_report(&records, &rules);
        let (yes, no) = (report[0].1.unwrap(), report[0].2.unwrap());
        assert_eq!((yes.avg, yes.games, no.avg, no.games), (2.0, 2, 7.0, 1));
        assert!(format_report(&records, &rules).contains("2.00 (2) / 7.00 (1)"));
        assert!(format_report(&[], &[]).contains("--import"));
    }
}
