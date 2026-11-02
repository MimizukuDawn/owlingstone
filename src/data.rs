//! The data files (`data/*.yaml`, the built-in card list and the changelog), built into the exe, and
//! read from a `data/` folder instead when there is one (see `paths::data_dir`), with reload on change.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::{paths, season::Season};

pub static CARDS_FALLBACK: &[u8] = include_bytes!("../data/cards-fallback.json.gz");
pub const CHANGELOG: &str = include_str!("../CHANGELOG.md");

/// The YAML files: name and built-in content.
pub const FILES: &[(&str, &str)] = &[
    ("comps.yaml", include_str!("../data/comps.yaml")),
    ("rules.yaml", include_str!("../data/rules.yaml")),
    ("heroes.yaml", include_str!("../data/heroes.yaml")),
    ("hero_curves.yaml", include_str!("../data/hero_curves.yaml")),
    ("hero_comps.yaml", include_str!("../data/hero_comps.yaml")),
    ("economy.yaml", include_str!("../data/economy.yaml")),
    ("trinkets.yaml", include_str!("../data/trinkets.yaml")),
    ("season.yaml", include_str!("../data/season.yaml")),
];

/// How often the files' modification times are checked.
const CHECK_EVERY: Duration = Duration::from_secs(3);

fn path_of(name: &str) -> Option<PathBuf> {
    paths::data_dir().map(|d| d.join(name)).filter(|p| p.is_file())
}

/// The text of a data file: from the data folder if it has it, else the built-in copy.
pub fn read(name: &str) -> String {
    if let Some(path) = path_of(name) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return text;
        }
    }
    FILES.iter().find(|(n, _)| *n == name).map(|(_, t)| t.to_string()).unwrap_or_default()
}

/// Modification times of the given files in the data folder (None = built-in copy).
pub fn stamps(names: &[&str]) -> Vec<Option<SystemTime>> {
    names
        .iter()
        .map(|n| path_of(n).and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()))
        .collect()
}

/// A value built from some data files, rebuilt when one of them (or the given extra key) changes,
/// checked at most every CHECK_EVERY.
pub struct Watched<T> {
    files: &'static [&'static str],
    state: Mutex<Option<WatchedState<T>>>,
}

struct WatchedState<T> {
    stamps: Vec<Option<SystemTime>>,
    extra: u64,
    checked: Instant,
    value: Arc<T>,
}

impl<T> Watched<T> {
    pub const fn new(files: &'static [&'static str]) -> Self {
        Watched { files, state: Mutex::new(None) }
    }

    /// The current value; `extra` is another version number it depends on (e.g. the card database).
    pub fn get(&self, extra: u64, build: impl FnOnce() -> T) -> Arc<T> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            if s.extra == extra && s.checked.elapsed() < CHECK_EVERY {
                return s.value.clone();
            }
            let now = stamps(self.files);
            s.checked = Instant::now();
            if s.extra == extra && now == s.stamps {
                return s.value.clone();
            }
        }
        let stamps = stamps(self.files);
        let value = Arc::new(build());
        *state = Some(WatchedState { stamps, extra, checked: Instant::now(), value: value.clone() });
        value
    }
}

static SEASON: Watched<Season> = Watched::new(&["season.yaml"]);

/// Season facts (data/season.yaml); defaults if the file is broken.
pub fn season() -> Arc<Season> {
    SEASON.get(0, || serde_yaml_ng::from_str(&read("season.yaml")).unwrap_or_default())
}

/// The lines of CHANGELOG.md for one version (the "- ..." items under "## <version>").
pub fn whats_new(version: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut inside = false;
    for line in CHANGELOG.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            inside = head.split_whitespace().next() == Some(version);
        } else if inside {
            if let Some(item) = line.trim().strip_prefix("- ") {
                lines.push(item.to_string());
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    #[test]
    fn built_in_files_parse() {
        for (name, text) in super::FILES {
            let parsed: Result<serde_yaml_ng::Value, _> = serde_yaml_ng::from_str(text);
            assert!(parsed.is_ok(), "{name}: {parsed:?}");
        }
        assert_eq!(super::season().lobby_tribes, 5);
    }

    #[test]
    fn whats_new_reads_one_version() {
        let lines = super::whats_new("1.9.0");
        assert!(lines.iter().any(|l| l.contains("Signed updates")));
        assert!(super::whats_new("0.0.1").is_empty());
    }
}
