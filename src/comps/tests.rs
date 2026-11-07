use super::*;
use crate::cards;
use crate::data;
use crate::knowledge::{Cond, Effect, OneOrMany};

fn know() -> Knowledge {
    Knowledge::build(&cards::test_db(), &|n| data::read(n))
}

fn cards_named(names: &[&str]) -> Vec<Arc<Card>> {
    let db = cards::test_db();
    let pool: Vec<Arc<Card>> = db.pool_minions(false).into_iter().chain(db.pool_spells(false)).collect();
    names.iter().map(|n| pool.iter().find(|c| c.name == *n).unwrap_or_else(|| panic!("{n}")).clone()).collect()
}

fn lobby() -> Vec<String> {
    ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"].iter().map(|s| s.to_string()).collect()
}

fn best(k: &Knowledge, board: &[Arc<Card>], hand: &[Arc<Card>], rules: &[Rule], ctx: &Ctx) -> Vec<String> {
    match_comps(&k.comps, board, hand, rules, ctx, 3, 0).iter().map(|m| m.comp.id.clone()).collect()
}

fn prefer(comp: &str, effect: Effect, cond: Cond) -> Vec<Rule> {
    vec![Rule { comps: vec![comp.into()], effect, cond, bonus: 3.0, note: String::new() }]
}

fn cthun() -> Cond {
    Cond { deity: Some("C'Thun".into()), ..Default::default() }
}

#[test]
fn comps_file_valid() {
    let k = know();
    assert!(k.comps.len() >= 25);
    let mut ids: Vec<&str> = k.comps.iter().map(|c| c.id.as_str()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), k.comps.len());
    assert!(k.comps.iter().filter(|c| c.id != "aberration-apm").all(|c| c.stats.avg.is_some()));
}

#[test]
fn comps_filtered_by_lobby_races() {
    let k = know();
    let l = lobby();
    let ids: Vec<&str> = available_comps(&k.comps, Some(&l)).iter().map(|c| c.id.as_str()).collect();
    for id in ["aberration-discard", "aberration-deathrattle", "neutral-tea-set"] {
        assert!(ids.contains(&id), "{id}");
    }
    assert!(!ids.contains(&"aberration-apm") && !ids.contains(&"pirate-discover"));
}

#[test]
fn board_detection_core_cards_win() {
    let k = know();
    let board = cards_named(&["Mysterious K'Thir", "Nightmare Corroder", "Cutthroat K'Thir"]);
    let m = match_comps(&k.comps, &board, &[], &[], &Ctx::default(), 3, 0);
    assert_eq!(m[0].comp.id, "aberration-discard");
    assert!(m[0].missing_core.iter().any(|c| c.name == "Harbinger Aph'lass"));
}

#[test]
fn hand_counts_less_than_board() {
    let k = know();
    let board = cards_named(&["Auto Reveille", "Spark Snapper"]);
    let hand = cards_named(&["Resourceful Robot", "Utility Drone", "Drakkari Enchanter"]);
    assert_eq!(best(&k, &board, &hand, &[], &Ctx::default())[0], "mech-apm");
    let all: Vec<Arc<Card>> = board.iter().chain(&hand).cloned().collect();
    assert_eq!(best(&k, &all, &[], &[], &Ctx::default())[0], "mech-volumizer");
}

#[test]
fn rules_hide_and_break_ties() {
    let k = know();
    let ctx = Ctx { races: Some(lobby()), deity: Some("C'Thun".into()), ..Default::default() };
    let board = cards_named(&["Nightmare Corroder"]); // add-on of both Discard and Deathrattle: a tie
    let r = prefer("aberration-deathrattle", Effect::PreferIf, cthun());
    assert_eq!(best(&k, &board, &[], &r, &ctx)[0], "aberration-deathrattle");
    let r = prefer("aberration-discard", Effect::PreferIf, cthun());
    assert_eq!(best(&k, &board, &[], &r, &ctx)[0], "aberration-discard");
    let hide = prefer(
        "aberration-deathrattle",
        Effect::NeverIf,
        Cond { lobby_has: Some(OneOrMany::Many(vec!["BEAST".into()])), ..Default::default() },
    );
    let board = cards_named(&["Faceless Converter"]);
    assert!(!best(&k, &board, &[], &hide, &ctx).contains(&"aberration-deathrattle".to_string()));
    // Tribes not sure yet: the rule hides nothing.
    let unsure = Ctx { races: None, ..ctx };
    assert!(best(&k, &board, &[], &hide, &unsure).contains(&"aberration-deathrattle".to_string()));
}

#[test]
fn rule_bonus_does_not_beat_a_real_match() {
    let k = know();
    let board = cards_named(&["Spark Snapper", "The Shadow of Doubt", "Forest Rover", "Futurefin"]);
    let r = prefer("aberration-deathrattle", Effect::PreferIf, cthun());
    let ctx = Ctx { deity: Some("C'Thun".into()), ..Default::default() };
    assert_eq!(match_comps(&k.comps, &board, &[], &r, &ctx, 3, 15)[0].comp.id, "mech-apm");
}

#[test]
fn lobby_comps_sorted_by_preference_then_placement() {
    let mut k = know();
    let ctx = Ctx { races: Some(lobby()), ..Default::default() };
    let ids: Vec<String> = lobby_comps(&k, &ctx).iter().map(|c| c.id.clone()).collect();
    assert!(ids[0] == "dragon-shield" || ids[0] == "neutral-tea-set", "{ids:?}");
    // Last = the worst average place of the lobby's comps (all have stats).
    let worst = lobby_comps(&k, &ctx)
        .into_iter()
        .max_by(|a, b| a.stats.avg.partial_cmp(&b.stats.avg).unwrap())
        .map(|c| c.id.clone())
        .unwrap();
    assert_eq!(ids.last().unwrap(), &worst);
    k.rules = prefer(&worst, Effect::PreferIf, Cond::default());
    assert_eq!(lobby_comps(&k, &ctx)[0].id, worst);
}

#[test]
fn race_fit_alone_is_not_a_comp() {
    let k = know();
    assert!(best(&k, &cards_named(&["Brain Rotter", "Underrot Spawn"]), &[], &[], &Ctx::default()).is_empty());
}

#[test]
fn board_profile_counts_races_and_keywords() {
    let (races, kw) = board_profile(&cards_named(&["Ghastcoiler", "Sewer Lord", "Mummifier"]));
    assert_eq!(races.len(), 2);
    assert_eq!((races["BEAST"], races["UNDEAD"]), (2, 1));
    assert_eq!(kw["Deathrattle"], 3);
}

#[test]
fn comp_tier_letters() {
    let k = know();
    let c = k.comp("aberration-discard").unwrap();
    assert_eq!(comp_tier(&k, c, None, 0.0).0, "S"); // 3.23
    let apm = k.comp("aberration-apm").unwrap();
    assert_eq!(comp_tier(&k, apm, None, 0.0).0, "?");
    // A 3-point preference moves 3.45 to 3.30: still S.
    let (letter, avg) = comp_tier(&k, c, None, -5.0);
    assert_eq!((letter, avg), ("A", 3.48));
    assert!(!hero_best_comps(&k, "Thorim, Stormlord", None, 2).is_empty());
}
