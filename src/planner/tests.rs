use super::*;
use crate::advice::ShopPick;
use crate::data;

fn know() -> Knowledge {
    Knowledge::build(&cards::test_db(), &|n| data::read(n))
}

fn cards_named(names: &[&str]) -> Vec<Arc<Card>> {
    let db = cards::test_db();
    let pool: Vec<Arc<Card>> = db.pool_minions(true).into_iter().chain(db.pool_spells(false)).collect();
    names.iter().map(|n| pool.iter().find(|c| c.name == *n).unwrap_or_else(|| panic!("{n}")).clone()).collect()
}

fn mm(card_id: &str, attack: i64, health: i64) -> MinionSnap {
    MinionSnap { card_id: card_id.into(), ..filler(attack, health, 1) }
}

fn sim(win: f64, loss: f64, taken: f64, lethal: f64) -> SimResult {
    let dealt = 5.0;
    SimResult {
        runs: 100,
        win,
        tie: 100.0 - win - loss,
        loss,
        dealt,
        taken,
        avg_dealt: dealt * win / 100.0,
        avg_taken: taken * loss / 100.0,
        lethal: 0.0,
        lethal_me: lethal,
        unknown: vec![],
        first: None,
    }
}

fn even() -> SimResult {
    sim(50.0, 50.0, 5.0, 0.0)
}

fn std_profile(k: &Knowledge, turn: i64) -> Profile {
    hero_profile(k, None, &[], turn)
}

fn plan(k: &Knowledge) -> Plan {
    Plan {
        turn: 6,
        tier: 3,
        gold: 8,
        health: Some(30),
        cap: Some(10),
        upgrade_cost: Some(7),
        reroll_cost: 1,
        opponent: None,
        opp_tier: 3,
        opp_health: None,
        basis: String::new(),
        opp_power: 0,
        my_power: 0,
        strength: 50.0,
        base: even(),
        per_card: vec![],
        power_gain: vec![],
        spend: Buys { buys: vec![], sim: even() },
        level: Some(Buys { buys: vec![], sim: even() }),
        team_size: 4,
        board_size: 4,
        weakest: vec![],
        profile: std_profile(k, 6),
        budget: 8,
        team_ids: vec![],
    }
}

fn weak(id: &str, name: &str, power: i64) -> Weak {
    Weak { base_id: id.into(), name: name.into(), power, golden: false }
}

#[test]
fn old_board_grows_with_the_turns() {
    let seen = vec![mm("BG31_803", 3, 3), mm("", 4, 5)];
    let last = Fight::for_tests(5, 3, seen.clone());
    let (board, basis) = expected_board(9, 4, Some(&last));
    assert_eq!(board[0].card_id, "BG31_803");
    assert_eq!(board.len(), 7);
    assert!(board_power(&board) > 10 * board_power(&seen));
    assert!(basis.contains("turn 5"));
    let now = Fight::for_tests(9, 4, seen.clone());
    assert_eq!(board_power(&expected_board(9, 4, Some(&now)).0), board_power(&seen));
}

#[test]
fn unknown_opponent_gets_a_typical_board() {
    let (board, basis) = expected_board(8, 4, None);
    assert!(basis.contains("typical"));
    let t = typical_power(8, 4) * TYPICAL_BOOST.exp();
    assert!((board_power(&board) as f64 - t).abs() < 0.1 * t);
    let s = strength_percentile(typical_power(8, 4).round() as i64, 8, 4);
    assert!(49.0 < s && s < 51.0, "{s}");
    assert!((erf(0.5) - 0.520_499_877_8).abs() < 1e-6);
}

#[test]
fn find_chance_grows_with_wanted_cards() {
    let db = cards::test_db();
    let pool: Vec<Arc<Card>> = db.pool_minions(false).into_iter().filter(|c| c.tier.is_some_and(|t| t <= 2)).take(40).collect();
    let one = find_chance(&pool[..1], 2, &pool);
    let three = find_chance(&pool[..3], 2, &pool);
    assert!(0.0 < one && one < three && three < 1.0);
    assert_eq!(find_chance(&[], 2, &pool), 0.0);
}

#[test]
fn level_on_curve_when_it_costs_nothing() {
    let k = know();
    let p = Plan { turn: 7, profile: std_profile(&k, 7), ..plan(&k) };
    let (ok, why) = level_advice(&p, 0.0);
    assert_eq!(ok, Some(true));
    assert!(why.contains("Tier 4"), "{why}");
}

#[test]
fn dont_level_into_lethal() {
    let k = know();
    let p = Plan {
        health: Some(6),
        level: Some(Buys { buys: vec![], sim: sim(0.0, 100.0, 8.0, 80.0) }),
        spend: Buys { buys: vec!["x".into()], sim: sim(60.0, 30.0, 4.0, 5.0) },
        ..plan(&k)
    };
    let (ok, why) = level_advice(&p, 0.0);
    assert_eq!(ok, Some(false));
    assert!(why.contains("die"));
}

#[test]
fn cannot_level() {
    let k = know();
    assert_eq!(level_advice(&Plan { gold: 5, budget: 5, level: None, ..plan(&k) }, 0.0).0, None);
    assert_eq!(level_advice(&Plan { upgrade_cost: None, ..plan(&k) }, 0.0).1, "max tier");
}

#[test]
fn turn_plan_buys_rolls_and_sells() {
    let k = know();
    let c = cards_named(&["Auto Reveille", "Glambot"]);
    let (core, fill) = (&c[0], &c[1]);
    let p = Plan {
        gold: 4,
        budget: 4,
        upgrade_cost: None,
        level: None,
        team_size: 7,
        per_card: vec![(core.id.clone(), sim(70.0, 20.0, 5.0, 0.0)), (fill.id.clone(), even())],
        power_gain: vec![(core.id.clone(), 5), (fill.id.clone(), 1)],
        weakest: vec![weak("PROTECTED", "Pair piece", 2), weak("BGS_004", "Wrath Weaver", 4)],
        ..plan(&k)
    };
    let picks = [ShopPick { card: core.clone(), value: 3.0, reasons: vec!["core Mech APM".into()] }];
    let protect: HashSet<String> = ["PROTECTED".to_string()].into();
    let out = turn_plan(&k, &p, &picks, &[], &[], &protect);
    assert_eq!(out.buy[0].card.id, core.id);
    assert!(out.buy[0].reasons.contains(&"core Mech APM".to_string()));
    assert_eq!(out.plan_buys.iter().map(|c| c.id.clone()).collect::<Vec<_>>(), [core.id.as_str()]);
    assert_eq!(out.sell.as_deref(), Some("Wrath Weaver (weakest)"));
    assert!(out.roll.unwrap().starts_with("Then roll 2×"));
}

#[test]
fn turn_plan_nothing_to_buy_means_roll() {
    let k = know();
    let p = Plan { gold: 7, budget: 7, upgrade_cost: None, level: None, ..plan(&k) };
    let out = turn_plan(&k, &p, &[], &[], &[], &HashSet::new());
    assert!(out.buy.is_empty());
    assert!(out.roll.unwrap().contains("roll up to 4×"));
}

#[test]
fn danger_from_lethal() {
    let k = know();
    let p = Plan { base: sim(0.0, 100.0, 12.0, 60.0), health: Some(10), ..plan(&k) };
    assert_eq!(turn_plan(&k, &p, &[], &[], &[], &HashSet::new()).danger.level, "critical");
}

#[test]
fn economy_counts_more_early() {
    let k = know();
    let accord = &cards_named(&["Accord-o-Tron"])[0];
    let (early, why) = economy_value(&k, &accord.id, 3);
    assert!(early > economy_value(&k, &accord.id, 12).0 && economy_value(&k, &accord.id, 12).0 > 0.0);
    assert!(why.contains("gold"));
    let weaver = &cards_named(&["Wrath Weaver"])[0];
    assert_eq!(economy_value(&k, &weaver.id, 3), (0.0, String::new()));
    let raw: std::collections::HashMap<String, serde_yaml_ng::Value> = serde_yaml_ng::from_str(&data::read("economy.yaml")).unwrap();
    assert_eq!(k.economy.len(), raw.len());
    assert!(raw.len() > 40);
}

#[test]
fn economy_card_beats_vanilla_buy() {
    let k = know();
    let c = cards_named(&["Accord-o-Tron", "Wrath Weaver"]);
    let p = Plan {
        turn: 5,
        per_card: vec![(c[0].id.clone(), even()), (c[1].id.clone(), even())],
        power_gain: vec![(c[0].id.clone(), 6), (c[1].id.clone(), 6)],
        ..plan(&k)
    };
    let out = turn_plan(&k, &p, &[], &[], &[], &HashSet::new());
    assert_eq!(out.buy[0].card.id, c[0].id);
    assert!(out.buy[0].reasons.iter().any(|r| r.contains("economy")));
}

#[test]
fn sell_used_battlecry_and_keep_gold_engines() {
    let k = know();
    let c = cards_named(&["Southsea Busker", "Accord-o-Tron", "Wrath Weaver"]);
    let board = vec![weak(&c[1].id, "Accord-o-Tron", 4), weak(&c[2].id, "Wrath Weaver", 5), weak(&c[0].id, "Southsea Busker", 6)];
    let (name, why) = sell_choice(&k, &board, &HashSet::new(), 6, 3).unwrap();
    assert_eq!(name, "Southsea Busker");
    assert!(why.contains("Battlecry"));
    assert_eq!(sell_choice(&k, &board[..2], &HashSet::new(), 6, 3).unwrap().0, "Wrath Weaver");
}

#[test]
fn hero_curves_file_names_are_real_heroes() {
    let k = know();
    for name in k.hero_curves.keys() {
        assert!(k.heroes.contains_key(name), "{name}");
    }
}

fn power(name: &str, cost: i64, exhausted: bool) -> Power {
    Power { name: name.into(), card_id: "X".into(), cost, exhausted }
}

#[test]
fn rafaam_curve_and_hero_power_gold() {
    let k = know();
    let p = hero_profile(&k, Some("Arch-Villain Rafaam"), &[power("I'll Take That!", 1, false)], 4);
    assert_eq!((p.curve[&3], p.curve[&4], p.curve[&6]), (1, 2, 4));
    assert_eq!((p.reserve, p.power.as_deref()), (1, Some("I'll Take That!")));
    assert_eq!(hero_profile(&k, Some("Arch-Villain Rafaam"), &[power("I'll Take That!", 1, true)], 4).reserve, 0);
    let std = hero_profile(&k, Some("Unknown Hero"), &[], 4);
    assert_eq!((std.curve[&2], std.curve[&5], std.reserve), (2, 3, 0));
    assert_eq!(hero_profile(&k, Some("Unknown Hero"), &[power("Paid", 2, false)], 3).reserve, 2);
    assert_eq!(hero_profile(&k, Some("Unknown Hero"), &[power("Paid", 2, false)], 2).reserve, 0);
    assert_eq!(hero_profile(&k, Some("Zephrys, the Great"), &[power("Three Wishes", 3, false)], 8).reserve, 0);
}

#[test]
fn hero_curve_changes_level_advice() {
    let k = know();
    let std = Plan { turn: 3, tier: 1, gold: 5, upgrade_cost: Some(3), budget: 5, profile: hero_profile(&k, Some("X"), &[], 3), ..plan(&k) };
    let raf = Plan {
        turn: 3,
        tier: 1,
        gold: 5,
        upgrade_cost: Some(3),
        budget: 4,
        profile: hero_profile(&k, Some("Arch-Villain Rafaam"), &[power("I'll Take That!", 1, false)], 3),
        ..plan(&k)
    };
    assert!(level_advice(&std, 0.0).1.ends_with(": on curve"), "{}", level_advice(&std, 0.0).1);
    let late = Plan { turn: 5, tier: 1, gold: 5, upgrade_cost: Some(3), budget: 5, profile: std_profile(&k, 5), ..plan(&k) };
    assert!(level_advice(&late, 0.0).1.contains("behind curve"));
    assert!(level_advice(&raf, 0.0).1.contains("ahead of Arch-Villain Rafaam's curve"));
    let out = turn_plan(&k, &raf, &[], &[], &[], &HashSet::new());
    assert!(out.power.unwrap().starts_with("Use I'll Take That! (1 gold"));
    assert!(out.hero_note.contains("Rafaam curve"));
}

#[test]
fn millhouse_buys_with_2_gold_minions() {
    let k = know();
    let c = cards_named(&["Wrath Weaver", "Glambot", "Annoy-o-Module"]);
    let p = Plan {
        gold: 6,
        budget: 6,
        upgrade_cost: None,
        level: None,
        per_card: c.iter().map(|x| (x.id.clone(), sim(60.0, 30.0, 5.0, 0.0))).collect(),
        power_gain: c.iter().map(|x| (x.id.clone(), 4)).collect(),
        profile: hero_profile(&k, Some("Millhouse Manastorm"), &[], 6),
        ..plan(&k)
    };
    assert_eq!(turn_plan(&k, &p, &[], &[], &[], &HashSet::new()).plan_buys.len(), 3);
}

#[test]
fn hoggarr_likes_pirates() {
    let k = know();
    let c = cards_named(&["Southsea Busker", "Wrath Weaver"]);
    let p = Plan {
        per_card: c.iter().map(|x| (x.id.clone(), even())).collect(),
        power_gain: c.iter().map(|x| (x.id.clone(), 4)).collect(),
        profile: hero_profile(&k, Some("Cap'n Hoggarr"), &[], 6),
        ..plan(&k)
    };
    let out = turn_plan(&k, &p, &[], &[], &[], &HashSet::new());
    assert_eq!(out.buy[0].card.id, c[0].id);
    assert!(out.buy[0].reasons.iter().any(|r| r.contains("Cap'n Hoggarr")));
}

#[test]
fn board_synergy_tribe_and_mechanics() {
    let pup = &cards_named(&["Wolf Pup"])[0];
    assert_eq!(board_synergy(pup, &cards_named(&["Headhunter Gryphon"])).0, 0.0);
    let gearfin = &cards_named(&["Gearfin"])[0];
    let (pts, why) = board_synergy(gearfin, &cards_named(&["Vicious Mindslasher"]));
    assert!(pts > 0.0 && why.contains("Tavern spell"), "{why}");
    assert_eq!(board_synergy(&cards_named(&["Wrath Weaver"])[0], &cards_named(&["Annoy-o-Module"])).0, 0.0);
}

#[test]
fn synergy_counts_in_buy_value() {
    let k = know();
    let c = cards_named(&["Gearfin", "Wrath Weaver"]);
    let p = Plan {
        per_card: c.iter().map(|x| (x.id.clone(), even())).collect(),
        power_gain: c.iter().map(|x| (x.id.clone(), 4)).collect(),
        team_ids: vec![cards_named(&["Vicious Mindslasher"])[0].id.clone()],
        ..plan(&k)
    };
    let out = turn_plan(&k, &p, &[], &[], &[], &HashSet::new());
    assert_eq!(out.buy[0].card.id, c[0].id);
    assert!(out.buy[0].reasons.iter().any(|r| r.contains("synergy now")));
}

#[test]
fn hero_best_comps_and_lobby_order() {
    use crate::comps::{hero_best_comps, lobby_comps};
    use crate::rules::Ctx;
    let k = know();
    for id in k.hero_comps.keys() {
        assert!(k.comp(id).is_some(), "{id}");
    }
    let best = hero_best_comps(&k, "Vanndar Stormpike", None, 3);
    assert_eq!((best[0].0.id.as_str(), best[0].1), ("neutral-tea-set", 2.57));
    let lobby: Vec<String> = ["MECHANICAL", "BEAST", "DRAGON", "MURLOC", "PIRATE"].iter().map(|s| s.to_string()).collect();
    let mech = hero_best_comps(&k, "Vanndar Stormpike", Some(&lobby), 3);
    assert!(mech.iter().all(|(c, _, _)| c.races.iter().all(|r| lobby.contains(r))));
    let plain: Vec<String> = lobby_comps(&k, &Ctx { races: Some(lobby.clone()), ..Default::default() }).iter().map(|c| c.id.clone()).collect();
    let with_hero: Vec<String> = lobby_comps(&k, &Ctx { races: Some(lobby), hero: Some("Tras'tath, Soul Parasite".into()), ..Default::default() })
        .iter()
        .map(|c| c.id.clone())
        .collect();
    assert_eq!(with_hero[0], "dragon-shield");
    assert_ne!(plain, with_hero);
}

#[test]
fn guessed_board_comes_in_several_strengths() {
    let (boards, _) = expected_boards(8, 4, None);
    assert_eq!(boards.len(), 7);
    assert_eq!(boards[3], expected_board(8, 4, None).0);
    assert!(boards.windows(2).all(|w| board_power(&w[0]) < board_power(&w[1])));
    let now = Fight::for_tests(9, 4, vec![mm("", 4, 5)]);
    assert_eq!(expected_boards(9, 4, Some(&now)).0.len(), 1); // seen during the fight: known
}
