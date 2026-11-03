//! settings.json in the user folder (same file and keys as the Python versions). Unknown keys are kept.

use serde_json::{Map, Value};

use crate::paths;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    values: Map<String, Value>,
}

/// Furthest the side panels can be made narrower / wider, in pixels.
pub const PANEL_DELTA_MAX: i64 = 160;

impl Settings {
    fn path() -> std::path::PathBuf {
        paths::user_dir().join("settings.json")
    }

    pub fn load() -> Settings {
        let values = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        Settings { values }
    }

    /// Writes the file (through a temporary file, so a crash never leaves half a file).
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&Value::Object(self.values.clone())).unwrap_or_default())?;
        std::fs::rename(tmp, path)
    }

    pub fn flag(&self, key: &str, default: bool) -> bool {
        self.values.get(key).and_then(Value::as_bool).unwrap_or(default)
    }

    pub fn set_flag(&mut self, key: &str, on: bool) {
        self.values.insert(key.into(), Value::Bool(on));
    }

    pub fn text(&self, key: &str) -> Option<String> {
        self.values.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
    }

    pub fn set_text(&mut self, key: &str, value: Option<&str>) {
        match value {
            Some(v) => self.values.insert(key.into(), Value::String(v.into())),
            None => self.values.remove(key),
        };
    }

    pub fn number(&self, key: &str, default: i64) -> i64 {
        self.values.get(key).and_then(Value::as_i64).unwrap_or(default)
    }

    pub fn set_number(&mut self, key: &str, n: i64) {
        self.values.insert(key.into(), Value::from(n));
    }

    // Named settings (defaults as in the Python versions).
    pub fn hide_when_unfocused(&self) -> bool {
        self.flag("hide_when_unfocused", true)
    }
    pub fn card_art(&self) -> bool {
        self.flag("card_art", true)
    }
    /// Mouse over a portrait of the game's leaderboard: that player's last board.
    pub fn board_on_hover(&self) -> bool {
        self.flag("board_on_hover", true)
    }
    /// Odds over the game's choices (hero pick, Trinket offer, Discover).
    pub fn choice_odds(&self) -> bool {
        self.flag("choice_odds", true)
    }
    pub fn lobby_panel(&self) -> bool {
        self.flag("lobby_panel", true)
    }
    /// The win chance banner at the top. Off by default when another tracker shows its own
    /// (see `top_odds_default`).
    pub fn top_odds(&self, default: bool) -> bool {
        self.flag("top_odds", default)
    }
    pub fn panel_delta(&self) -> i64 {
        self.number("panel_delta", 0).clamp(-PANEL_DELTA_MAX, PANEL_DELTA_MAX)
    }
    pub fn hearthstone_dir(&self) -> Option<String> {
        self.text("hearthstone_dir")
    }
    pub fn last_version(&self) -> Option<String> {
        self.text("last_version")
    }
    /// Right panel tab: "Comps" or "Lobby" (as the Python versions wrote it).
    pub fn hud_tab(&self) -> String {
        match self.text("hud_tab") {
            Some(t) if t.eq_ignore_ascii_case("lobby") => "Lobby".into(),
            _ => "Comps".into(),
        }
    }
    /// Keys of the right panel's folded sections (e.g. "buy", "turn plan").
    pub fn folded(&self) -> std::collections::BTreeSet<String> {
        self.values
            .get("folded")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    }
    pub fn set_folded(&mut self, keys: &std::collections::BTreeSet<String>) {
        self.values.insert("folded".into(), Value::from(keys.iter().cloned().collect::<Vec<_>>()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_unknown_keys_and_defaults() {
        let s: Settings = Settings { values: serde_json::from_str(r#"{"card_art": false, "future": [1], "panel_delta": 900}"#).unwrap() };
        assert!(!s.card_art() && s.hide_when_unfocused() && s.top_odds(true) && !s.top_odds(false));
        assert_eq!(s.panel_delta(), PANEL_DELTA_MAX);
        let mut s2 = s.clone();
        s2.set_flag("lobby_panel", false);
        assert!(s2.values.contains_key("future"));
        assert_eq!(s.hud_tab(), "Comps");
        let mut f = Settings { values: serde_json::from_str(r#"{"folded": ["buy", "turn plan"], "hud_tab": "Lobby"}"#).unwrap() };
        assert_eq!(f.folded().into_iter().collect::<Vec<_>>(), ["buy", "turn plan"]);
        assert_eq!(f.hud_tab(), "Lobby");
        f.set_folded(&["comps".to_string()].into());
        assert_eq!(f.values["folded"], serde_json::json!(["comps"]));
    }
}
