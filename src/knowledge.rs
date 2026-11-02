//! What the app knows about the game besides the cards: comps, rules, hero and trinket stats, hero
//! curves, economy cards. All from the YAML files in data/ (built in, or read from a data folder and
//! reloaded when they change). A broken entry is skipped and reported in `errors`, never fatal.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::Deserialize;

use crate::cards::{self, Card, CardDb};
use crate::data::{self, Watched};

/// A YAML value that may be one string or a list of strings.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn list(&self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s.clone()],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}

// ------------------------------------------------------------------------------------------- comps
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct CompStats {
    #[serde(default)]
    pub avg: Option<f64>,
    #[serde(default)]
    pub games: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Comp {
    pub id: String,
    pub name: String,
    pub races: Vec<String>,
    pub core: Vec<Arc<Card>>,
    pub addons: Vec<Arc<Card>>,
    pub difficulty: String,
    pub when_to_commit: String,
    pub how_to_play: String,
    pub stats: CompStats,
}

impl Comp {
    pub fn has_core(&self, id: &str) -> bool {
        self.core.iter().any(|c| c.id == id)
    }
    pub fn has_addon(&self, id: &str) -> bool {
        self.addons.iter().any(|c| c.id == id)
    }
}

#[derive(Deserialize)]
struct RawComp {
    id: String,
    name: String,
    #[serde(default)]
    races: Vec<String>,
    #[serde(default)]
    core: Vec<String>,
    #[serde(default)]
    addons: Vec<String>,
    #[serde(default)]
    difficulty: String,
    #[serde(default)]
    when_to_commit: String,
    #[serde(default)]
    how_to_play: String,
    #[serde(default)]
    stats: Option<CompStats>,
}

// ------------------------------------------------------------------------------------------- rules
/// Conditions of a rule (all must be true; none = always).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Cond {
    pub lobby_has: Option<OneOrMany>,
    pub lobby_has_any: Option<OneOrMany>,
    pub lobby_lacks: Option<OneOrMany>,
    pub deity: Option<String>,
    pub hero: Option<String>,
    pub hero_power: Option<OneOrMany>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// The comp is offered only if the condition is true.
    OnlyIf,
    /// The comp is hidden if the condition is true.
    NeverIf,
    /// +bonus points when the condition is true.
    PreferIf,
    /// -bonus points when the condition is true.
    AvoidIf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub comps: Vec<String>,
    pub effect: Effect,
    pub cond: Cond,
    pub bonus: f64,
    pub note: String,
}

pub const DEFAULT_BONUS: f64 = 3.0;

#[derive(Deserialize)]
struct RawRule {
    comp: Option<OneOrMany>,
    only_if: Option<Option<Cond>>,
    never_if: Option<Option<Cond>>,
    prefer_if: Option<Option<Cond>>,
    avoid_if: Option<Option<Cond>>,
    bonus: Option<f64>,
    #[serde(default)]
    note: String,
}

// ------------------------------------------------------------------------------------ heroes & co
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
pub struct HeroStat {
    pub avg: f64,
    #[serde(default)]
    pub games: f64,
}

/// How to use a hero power in the plan (data/hero_curves.yaml).
#[derive(Debug, Clone, PartialEq)]
pub enum PowerRule {
    /// Automatic: keep the gold of a 1-2 gold power from turn 3.
    Auto,
    Never,
    Turns { from_turn: i64, until_turn: i64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeroCurve {
    /// Turn -> tier to reach by the end of that turn (marks only).
    pub curve: BTreeMap<i64, i64>,
    pub power: PowerRule,
    pub level_bias: f64,
    pub minion_cost: Option<i64>,
    pub buy_bonus: BTreeMap<String, f64>,
    pub pair_bonus: f64,
    pub note: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawPower {
    Word(String),
    Turns {
        #[serde(default)]
        from_turn: Option<i64>,
        #[serde(default)]
        until_turn: Option<i64>,
    },
}

#[derive(Deserialize)]
struct RawCurve {
    #[serde(default)]
    curve: BTreeMap<i64, i64>,
    power: Option<RawPower>,
    #[serde(default)]
    level_bias: f64,
    minion_cost: Option<i64>,
    #[serde(default)]
    buy_bonus: BTreeMap<String, f64>,
    #[serde(default)]
    pair_bonus: f64,
    #[serde(default)]
    note: String,
}

/// An economy / card-advantage card (data/economy.yaml).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Economy {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: f64,
    /// "gold" or "cards".
    #[serde(default)]
    pub kind: String,
    /// "board" (pays while on board), "play" (Battlecry), "sell".
    #[serde(default)]
    pub when: String,
    #[serde(default)]
    pub note: String,
}

/// One trinket of data/trinkets.yaml.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TrinketStat {
    pub id: String,
    pub name: String,
    /// "lesser" or "greater".
    pub kind: String,
    #[serde(default)]
    pub cost: i64,
    pub avg: f64,
    #[serde(default)]
    pub pick: f64,
    #[serde(default)]
    pub games: f64,
    #[serde(default)]
    pub races: Vec<String>,
    #[serde(default)]
    pub gives: Option<String>,
}

// --------------------------------------------------------------------------------------- the lot
#[derive(Debug, Default)]
pub struct Knowledge {
    pub comps: Vec<Comp>,
    pub rules: Vec<Rule>,
    /// Hero name -> average place.
    pub heroes: HashMap<String, HeroStat>,
    /// Comp id -> hero -> (average place with this comp, games).
    pub hero_comps: HashMap<String, HashMap<String, (f64, f64)>>,
    /// Hero or hero power name -> plan settings.
    pub hero_curves: HashMap<String, HeroCurve>,
    /// Card id -> economy info.
    pub economy: HashMap<String, Economy>,
    /// Card id -> trinket stats.
    pub trinkets: HashMap<String, TrinketStat>,
    /// Pool minions (Solo and Duos) by name.
    pub minion_by_name: HashMap<String, Arc<Card>>,
    /// Problems found in the files (shown to the user, the rest still works).
    pub errors: Vec<String>,
}

const FILES: &[&str] = &[
    "comps.yaml",
    "rules.yaml",
    "heroes.yaml",
    "hero_curves.yaml",
    "hero_comps.yaml",
    "economy.yaml",
    "trinkets.yaml",
    "season.yaml",
];
static KNOWLEDGE: Watched<Knowledge> = Watched::new(FILES);

/// The current knowledge (rebuilt when a data file or the card list changes).
pub fn get() -> Arc<Knowledge> {
    KNOWLEDGE.get(cards::generation(), || Knowledge::build(&cards::db(), &|name| data::read(name)))
}

fn parse<T: for<'de> Deserialize<'de> + Default>(name: &str, text: &str, errors: &mut Vec<String>) -> T {
    if text.trim().is_empty() {
        return T::default();
    }
    match serde_yaml_ng::from_str::<Option<T>>(text) {
        Ok(v) => v.unwrap_or_default(),
        Err(e) => {
            errors.push(format!("{name}: {e}"));
            T::default()
        }
    }
}

impl Knowledge {
    /// Builds everything from the file texts (`read(name)`).
    pub fn build(db: &CardDb, read: &dyn Fn(&str) -> String) -> Knowledge {
        let mut errors = Vec::new();
        let mut k = Knowledge::default();

        // Cards of the Solo pool by name (comps are Solo), and every pool minion by name.
        let solo: HashMap<String, Arc<Card>> = db
            .pool_minions(false)
            .into_iter()
            .chain(db.pool_spells(false))
            .map(|c| (c.name.clone(), c))
            .collect();
        k.minion_by_name = db.pool_minions(true).into_iter().map(|c| (c.name.clone(), c)).collect();

        let raw: Vec<serde_yaml_ng::Value> = parse("comps.yaml", &read("comps.yaml"), &mut errors);
        for (i, v) in raw.into_iter().enumerate() {
            let r: RawComp = match serde_yaml_ng::from_value(v) {
                Ok(r) => r,
                Err(e) => {
                    errors.push(format!("comps.yaml entry {}: {e}", i + 1));
                    continue;
                }
            };
            let mut find = |names: &[String]| -> Vec<Arc<Card>> {
                names
                    .iter()
                    .filter_map(|n| {
                        let c = solo.get(n).cloned();
                        if c.is_none() {
                            errors.push(format!("comps.yaml {}: unknown card '{n}'", r.id));
                        }
                        c
                    })
                    .collect()
            };
            let core = find(&r.core);
            let addons = find(&r.addons);
            k.comps.push(Comp {
                id: r.id.clone(),
                name: r.name,
                races: r.races,
                core,
                addons,
                difficulty: r.difficulty,
                when_to_commit: r.when_to_commit,
                how_to_play: r.how_to_play,
                stats: r.stats.unwrap_or_default(),
            });
        }

        let comp_ids: Vec<&str> = k.comps.iter().map(|c| c.id.as_str()).collect();
        let raw: Vec<serde_yaml_ng::Value> = parse("rules.yaml", &read("rules.yaml"), &mut errors);
        for (i, v) in raw.into_iter().enumerate() {
            let where_ = format!("rules.yaml rule {}", i + 1);
            let r: RawRule = match serde_yaml_ng::from_value(v) {
                Ok(r) => r,
                Err(e) => {
                    errors.push(format!("{where_}: {e}"));
                    continue;
                }
            };
            let comps = r.comp.map(|c| c.list()).unwrap_or_default();
            if comps.is_empty() {
                errors.push(format!("{where_}: 'comp' missing"));
            }
            for c in &comps {
                if !comp_ids.contains(&c.as_str()) {
                    errors.push(format!("{where_}: unknown comp '{c}'"));
                }
            }
            let effects = [
                (Effect::OnlyIf, r.only_if),
                (Effect::NeverIf, r.never_if),
                (Effect::PreferIf, r.prefer_if),
                (Effect::AvoidIf, r.avoid_if),
            ];
            let found: Vec<(Effect, Option<Cond>)> =
                effects.into_iter().filter_map(|(e, c)| c.map(|c| (e, c))).collect();
            if found.len() != 1 {
                errors.push(format!("{where_}: needs exactly one of only_if, never_if, prefer_if, avoid_if"));
                continue;
            }
            let (effect, cond) = found.into_iter().next().expect("one effect");
            k.rules.push(Rule { comps, effect, cond: cond.unwrap_or_default(), bonus: r.bonus.unwrap_or(DEFAULT_BONUS), note: r.note });
        }

        k.heroes = parse("heroes.yaml", &read("heroes.yaml"), &mut errors);
        k.hero_comps = parse::<HashMap<String, Option<HashMap<String, (f64, f64)>>>>(
            "hero_comps.yaml",
            &read("hero_comps.yaml"),
            &mut errors,
        )
        .into_iter()
        .map(|(c, h)| (c, h.unwrap_or_default()))
        .collect();

        let curves: HashMap<String, RawCurve> = parse("hero_curves.yaml", &read("hero_curves.yaml"), &mut errors);
        for (name, r) in curves {
            let power = match r.power {
                None => PowerRule::Auto,
                Some(RawPower::Word(w)) if w == "never" => PowerRule::Never,
                Some(RawPower::Word(w)) => {
                    errors.push(format!("hero_curves.yaml {name}: unknown power rule '{w}'"));
                    PowerRule::Auto
                }
                Some(RawPower::Turns { from_turn, until_turn }) => {
                    PowerRule::Turns { from_turn: from_turn.unwrap_or(1), until_turn: until_turn.unwrap_or(99) }
                }
            };
            k.hero_curves.insert(
                name,
                HeroCurve {
                    curve: r.curve,
                    power,
                    level_bias: r.level_bias,
                    minion_cost: r.minion_cost,
                    buy_bonus: r.buy_bonus,
                    pair_bonus: r.pair_bonus,
                    note: r.note,
                },
            );
        }

        let economy: HashMap<String, Economy> = parse("economy.yaml", &read("economy.yaml"), &mut errors);
        for (name, mut info) in economy {
            match k.minion_by_name.get(&name) {
                Some(c) => {
                    info.name = name;
                    k.economy.insert(c.id.clone(), info);
                }
                None => errors.push(format!("economy.yaml: unknown card '{name}'")),
            }
        }

        let trinkets: Vec<serde_yaml_ng::Value> = parse("trinkets.yaml", &read("trinkets.yaml"), &mut errors);
        for v in trinkets {
            match serde_yaml_ng::from_value::<TrinketStat>(v) {
                Ok(t) => {
                    k.trinkets.insert(t.id.clone(), t);
                }
                Err(e) => errors.push(format!("trinkets.yaml: {e}")),
            }
        }
        k.errors = errors;
        k
    }

    pub fn comp(&self, id: &str) -> Option<&Comp> {
        self.comps.iter().find(|c| c.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_files_load_without_errors() {
        let db = cards::test_db();
        let k = Knowledge::build(&db, &|name| data::read(name));
        assert!(k.errors.is_empty(), "{:?}", k.errors);
        assert!(k.comps.len() >= 20);
        assert!(k.comps.iter().all(|c| !c.core.is_empty()));
        assert!(!k.rules.is_empty() && !k.heroes.is_empty() && !k.trinkets.is_empty());
        assert!(!k.economy.is_empty() && !k.hero_curves.is_empty() && !k.hero_comps.is_empty());
        assert_eq!(k.hero_curves["Forest Warden Omu"].level_bias, 1.5);
    }

    #[test]
    fn broken_entries_are_reported_not_fatal() {
        let db = cards::test_db();
        let read = |name: &str| match name {
            "comps.yaml" => "- {id: a, name: A, core: [Not A Card]}\n- {id: b, name: B, core: []}".to_string(),
            "rules.yaml" => "- {comp: zzz, prefer_if: {}}\n- {comp: a, prefer_if: {}, avoid_if: {}}\n- {comp: b, only_if: {foo: 1}}".to_string(),
            _ => String::new(),
        };
        let k = Knowledge::build(&db, &read);
        assert_eq!(k.comps.len(), 2);
        assert!(k.errors.iter().any(|e| e.contains("Not A Card")));
        assert!(k.errors.iter().any(|e| e.contains("unknown comp 'zzz'")));
        assert!(k.errors.iter().any(|e| e.contains("exactly one")));
        assert!(k.errors.iter().any(|e| e.contains("rule 3")));
    }
}
