//! Simulator tests: base rules, card effects. Real fights of a game are tested in tests/game1.rs.

use super::*;
use crate::cards;

fn m(card_id: &str, attack: i64, health: i64) -> MinionSnap {
    MinionSnap {
        card_id: card_id.to_string(),
        attack,
        health,
        max_health: health,
        golden: false,
        tier: 1,
        races: vec![],
        taunt: false,
        ds: false,
        venom: false,
        poison: false,
        reborn: false,
        stealth: false,
        cleave: false,
        windfury: 0,
        data: (0, 0),
    }
}

fn plain(attack: i64, health: i64) -> MinionSnap {
    m("", attack, health)
}

fn odds_with(mine: Vec<MinionSnap>, theirs: Vec<MinionSnap>, f: impl FnOnce(&mut SimInput)) -> SimResult {
    let db = cards::test_db();
    let mut input = SimInput { mine, theirs, ..Default::default() };
    f(&mut input);
    simulate_with(&db, &input, 400, 10.0, Some(1))
}

fn odds(mine: Vec<MinionSnap>, theirs: Vec<MinionSnap>) -> SimResult {
    odds_with(mine, theirs, |_| {})
}

#[test]
fn empty_and_bigger_board() {
    let r = odds(vec![plain(3, 3)], vec![]);
    assert_eq!(r.win, 100.0);
    assert_eq!(r.dealt, 2.0); // hero tier + tier of the minion left
    assert_eq!(odds(vec![], vec![]).tie, 100.0);
    assert_eq!(odds(vec![plain(5, 5)], vec![plain(1, 1)]).win, 100.0);
}

#[test]
fn trade_is_a_tie() {
    assert_eq!(odds(vec![plain(2, 2)], vec![plain(2, 2)]).tie, 100.0);
}

#[test]
fn divine_shield_venomous_reborn() {
    assert_eq!(odds(vec![MinionSnap { ds: true, ..plain(2, 2) }], vec![plain(2, 2)]).win, 100.0);
    assert_eq!(odds(vec![MinionSnap { venom: true, ..plain(1, 1) }], vec![plain(1, 50)]).tie, 100.0);
    // Reborn: comes back with 1 Health and wins the fight
    let theirs = vec![MinionSnap { taunt: true, ..plain(1, 3) }, plain(0, 3)];
    assert_eq!(odds(vec![plain(3, 1)], theirs.clone()).loss, 100.0);
    assert_eq!(odds(vec![MinionSnap { reborn: true, ..plain(3, 1) }], theirs).win, 100.0);
}

#[test]
fn taunt_is_attacked_first() {
    // My 4/4 always hits the 3/10 Taunt (and dies on the 2nd hit), never the 0/1
    let r = odds(vec![plain(4, 4)], vec![MinionSnap { taunt: true, ..plain(3, 10) }, plain(0, 1)]);
    assert_eq!(r.loss, 100.0);
}

#[test]
fn deathrattle_summons() {
    let vermin = MinionSnap { taunt: true, ..m("BG31_803", 1, 1) }; // Deathrattle: summon a 2/2 Beetle
    assert_eq!(odds(vec![vermin.clone()], vec![plain(2, 2)]).tie, 100.0);
    let r = odds_with(vec![vermin], vec![plain(2, 2)], |i| i.counters[0].beetle = Some((2, 1)));
    assert_eq!(r.win, 100.0); // 4/3 Beetle thanks to the game's counter
}

#[test]
fn damage_cap_and_lethal() {
    let db = cards::test_db();
    let big: Vec<MinionSnap> = (0..7).map(|_| MinionSnap { tier: 6, ..plain(10, 10) }).collect();
    let mut input = SimInput { mine: big, my_tier: 6, their_health: Some(20), ..Default::default() };
    let r = simulate_with(&db, &input, 50, 10.0, Some(1));
    assert_eq!((r.dealt, r.lethal), (48.0, 100.0));
    input.cap = Some(15);
    let r = simulate_with(&db, &input, 50, 10.0, Some(1));
    assert_eq!((r.dealt, r.lethal), (15.0, 0.0));
}

#[test]
fn same_seed_same_odds() {
    let db = cards::test_db();
    let input = SimInput {
        mine: vec![plain(3, 4), plain(2, 6), plain(5, 1)],
        theirs: vec![plain(4, 4), plain(3, 3), plain(1, 7)],
        ..Default::default()
    };
    let a = simulate_with(&db, &input, 2000, 10.0, Some(9));
    let b = simulate_with(&db, &input, 2000, 10.0, Some(9));
    assert_eq!(a, b);
    assert_eq!(a.runs, 2000);
    assert!((a.win + a.tie + a.loss - 100.0).abs() < 1e-9);
}

#[test]
fn effect_tables_are_consistent() {
    for id in ["BG36_210", "BG27_017", "BG36_333"] {
        assert!(rally_of(id).unwrap().golden_in_text());
    }
    assert!(!rally_of("BG34_320").unwrap().golden_in_text());
    let db = cards::test_db();
    // Coded cards are never "unknown".
    for id in ["BG31_803", "BG36_200", "BG34_405", "BG35_814"] {
        if let Some(c) = db.get(id) {
            assert!(!is_unknown_effect(Some(c)), "{id}");
        }
    }
}
