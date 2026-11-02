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
