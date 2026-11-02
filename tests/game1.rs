//! Real game checks (private log in testdata/, never published: skipped when missing).

use std::path::PathBuf;

use owlingstone::combat::{fight_input, simulate};
use owlingstone::game::Game;
use owlingstone::powerlog::PowerLogParser;

fn game1() -> Option<Game> {
    unsafe { std::env::set_var("OWLINGSTONE_NO_CARD_DOWNLOAD", "1") };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/bg_game1.log");
    if !path.exists() {
        eprintln!("testdata/bg_game1.log missing: skipped");
        return None;
    }
    let mut p = PowerLogParser::new(true);
    p.feed_file(&path).unwrap();
    p.into_games().pop()
}

#[test]
fn game1_reads_like_python() {
    let Some(g) = game1() else { return };
    assert!(g.is_battlegrounds());
    assert_eq!(g.my_hero().unwrap().name(), "Thorim, Stormlord");
    assert_eq!(g.tavern_tier(), Some(6));
    assert!(g.is_over());
    assert_eq!(g.my_placement(), Some(6));
    let mut races = g.available_races();
    races.sort();
    assert_eq!(races, ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"]);
    assert!(g.races_complete());
    let heroes = g.lobby_heroes();
    assert_eq!(heroes.len(), 8);
    let mut places: Vec<i64> = heroes.iter().map(|h| h.int("PLAYER_LEADERBOARD_PLACE")).collect();
    places.sort();
    assert_eq!(places, (1..=8).collect::<Vec<_>>());
    assert_eq!(g.fights.len(), 10);
    assert_eq!(g.fights.iter().filter(|f| f.result == 'W').count(), 2);
}

#[test]
fn game1_fights_are_predicted() {
    let Some(g) = game1() else { return };
    let mut out = Vec::new();
    for f in &g.fights {
        let input = fight_input(&g, f).expect("frozen fight");
        let r = simulate(&input, 6000, 30.0, Some(1));
        let best = [('W', r.win), ('T', r.tie), ('L', r.loss)].into_iter().fold(('?', -1.0), |a, b| if b.1 > a.1 { b } else { a }).0;
        out.push(serde_json::json!({"turn": f.turn, "win": r.win, "tie": r.tie, "loss": r.loss, "result": f.result.to_string()}));
        assert_eq!(best, f.result, "turn {}: {r:?}", f.turn);
    }
    if let Ok(path) = std::env::var("OWLINGSTONE_DUMP_ODDS") {
        std::fs::write(path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
    }
}

fn game1_lines() -> Option<String> {
    unsafe { std::env::set_var("OWLINGSTONE_NO_CARD_DOWNLOAD", "1") };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/bg_game1.log");
    std::fs::read(&path).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

#[test]
fn game1_hero_options_follow_reroll_and_pick() {
    let Some(text) = game1_lines() else { return };
    let mut p = PowerLogParser::new(true);
    let mut seen: Vec<Vec<String>> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        p.feed_line(line);
        if i % 50 == 0 {
            if let Some(g) = p.game() {
                let mut opts: Vec<String> = g.hero_options().iter().map(|e| e.name()).collect();
                opts.sort();
                if !opts.is_empty() && !seen.contains(&opts) {
                    seen.push(opts);
                }
            }
        }
        if i > 3000 {
            break;
        }
    }
    let n = seen.len();
    assert_eq!(seen[n - 2], ["Dancin' Deryl", "Genn, Worgen King", "George the Fallen", "Rock Master Voone"]);
    assert_eq!(seen[n - 1], ["Genn, Worgen King", "George the Fallen", "Rock Master Voone", "Thorim, Stormlord"]);
    let g = p.game().unwrap();
    assert!(g.hero_options().is_empty());
    assert_eq!(g.my_hero().unwrap().name(), "Thorim, Stormlord");
}

#[test]
fn game1_plan_and_costs() {
    use owlingstone::planner::{Caches, fight_plan};
    let Some(text) = game1_lines() else { return };
    let mut p = PowerLogParser::new(true);
    for line in text.lines() {
        p.feed_line(line);
        let Some(g) = p.game() else { continue };
        if g.turn() == 6 && !g.in_combat() && g.shop().len() >= 3 && g.my_gold() == Some(8) {
            break;
        }
    }
    let g = p.game().unwrap();
    assert_eq!((g.upgrade_cost(), g.reroll_cost()), (Some(7), 1));
    let k = owlingstone::knowledge::get();
    let mut caches = Caches::default();
    let fp = fight_plan(&k, g, &mut caches).expect("a plan");
    assert_eq!(fp.gold, 8);
    assert!(fp.opponent.is_some() && fp.level.is_some());
    let db = owlingstone::cards::db();
    let mut ids: Vec<String> = fp.per_card.iter().map(|(i, _)| i.clone()).collect();
    let mut shop: Vec<String> = g.shop().iter().map(|e| db.base_id(&e.card_id).to_string()).collect();
    ids.sort();
    shop.sort();
    shop.dedup();
    assert_eq!(ids, shop);
    let again = fight_plan(&k, g, &mut caches).unwrap();
    assert!(std::sync::Arc::ptr_eq(&fp, &again)); // cached while nothing changes
}

#[test]
fn game1_combat_info() {
    use owlingstone::planner::{Caches, combat_info};
    let Some(mut g) = game1() else { return };
    let info = combat_info(&mut g, &mut Caches::default()).unwrap();
    assert_eq!(info.fights.len(), 10);
    assert_eq!(info.actual_wins, 2);
    let (w, _, _) = info.avg.unwrap();
    assert!((0.0..=100.0).contains(&w));
    assert!(info.now.as_ref().is_none_or(|n| !n.live));
}
