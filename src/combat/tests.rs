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

/// Runs `f` on a fight set up with these boards (nothing played yet).
fn with_fight(mine: Vec<MinionSnap>, theirs: Vec<MinionSnap>, f: impl FnOnce(&mut Fight)) {
    let db = cards::test_db();
    let ctx = Ctx {
        db: &db,
        beetle: [(2, 2); 2],
        lobster: [(0, 0); 2],
        deathrattle_pool: vec![],
        deity_id: None,
        deity_stats: [(0, 0); 2],
        knights: [0; 2],
        undead: [0; 2],
        gem: [(1, 1); 2],
        spell: [(0, 0); 2],
    };
    let protos = [
        mine.iter().map(|d| M::new(d, 0, &db)).collect::<Vec<_>>(),
        theirs.iter().map(|d| M::new(d, 1, &db)).collect::<Vec<_>>(),
    ];
    let mut rng = Rng::new(1);
    let mut fight = Fight::new(&protos, &ctx, &mut rng);
    f(&mut fight);
}

#[test]
fn patch_37_0_2_cards() {
    // Ship Master Eudora: Deathrattle +8/+8 (was +6/+6).
    let r = odds(
        vec![MinionSnap { taunt: true, ..m("BG33_828", 0, 1) }, plain(1, 1)],
        vec![plain(1, 9), plain(1, 9), plain(1, 9)],
    );
    assert_eq!(r.win, 100.0);
    // Stalwart Kodo: double its maximum stats (golden: triple), 3 times per fight.
    with_fight(vec![m("BG34_322", 2, 10)], vec![], |f| {
        let t = f.token("", 0).stats(1, 1);
        let i = f.summon(0, t, None, true).unwrap();
        assert_eq!((f.ms[i].atk, f.ms[i].hp), (5, 21));
        for _ in 0..3 {
            let t = f.token("", 0).stats(1, 1);
            f.summon(0, t, None, true);
        }
        let last = *f.sides[0].last().unwrap();
        assert_eq!(f.ms[last].atk, 1); // 4th summon: no more
    });
    with_fight(vec![MinionSnap { golden: true, ..m("BG34_322_G", 2, 10) }], vec![], |f| {
        let t = f.token("", 0).stats(1, 1);
        let i = f.summon(0, t, None, true).unwrap();
        assert_eq!((f.ms[i].atk, f.ms[i].hp), (7, 31));
    });
    // The Last One Standing: +10/+10, improving after each Rally.
    let beast = MinionSnap { races: vec!["BEAST".into()], ..plain(1, 1) };
    with_fight(vec![m("BG34_320", 10, 10), beast.clone()], vec![plain(0, 99)], |f| {
        let (me, b, target) = (f.sides[0][0], f.sides[0][1], f.sides[1][0]);
        f.rally(Rally::LastOne, 0, me, target);
        assert_eq!(f.ms[b].atk, 11);
        f.rally(Rally::LastOne, 0, me, target);
        assert_eq!(f.ms[b].atk, 31);
    });
    // ... starting from the amount the game shows.
    with_fight(vec![MinionSnap { data: (30, 0), ..m("BG34_320", 10, 10) }, beast], vec![plain(0, 99)], |f| {
        let (me, b, target) = (f.sides[0][0], f.sides[0][1], f.sides[1][0]);
        f.rally(Rally::LastOne, 0, me, target);
        assert_eq!(f.ms[b].atk, 31);
    });
    // Sanguine Refiner: Blood Gems +1/+1 more (was +1/+2).
    with_fight(vec![m("BG33_885", 4, 9)], vec![plain(0, 99)], |f| {
        let (me, target) = (f.sides[0][0], f.sides[1][0]);
        f.rally(Rally::Refiner, 0, me, target);
        assert_eq!(f.gem[0], (2, 2));
    });
}
