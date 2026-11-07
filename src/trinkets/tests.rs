use super::*;
use crate::game::Game;
use crate::{cards, data};

fn know() -> Knowledge {
    Knowledge::build(&cards::test_db(), &|n| data::read(n))
}

fn card(name: &str) -> Arc<Card> {
    let db = cards::test_db();
    let mut found: Vec<Arc<Card>> =
        db.by_id.values().filter(|c| c.name == name && c.tier.is_some_and(|t| t != 0) && c.id.starts_with("BG")).cloned().collect();
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found.iter().find(|c| c.is_pool_minion).cloned().unwrap_or_else(|| found[0].clone())
}

fn trinket(k: &Knowledge, name: &str, kind: &str) -> Arc<Card> {
    let t = k.trinket_list.iter().find(|t| t.name == name && t.kind == kind).unwrap();
    cards::test_db().get(&t.id).unwrap().clone()
}

fn game() -> Game {
    let mut g = Game::new();
    g.add_player(2, 20, true);
    g.add_player(9, 90, false);
    g.create_entity(1, "");
    g.game_entity_id = Some(1);
    g.set_tag(1, "TURN", "11"); // turn 6, recruit
    g.create_entity(20, "");
    g.create_entity(90, "");
    g
}

fn ent(g: &mut Game, eid: i64, card_id: &str, tags: &[(&str, &str)]) {
    g.create_entity(eid, card_id);
    for (t, v) in tags {
        g.set_tag(eid, t, v);
    }
}

#[test]
fn offer_read_from_log_state() {
    let _ = cards::test_db();
    let mut g = game();
    let trinket = [("CONTROLLER", "2"), ("ZONE", "SETASIDE"), ("CARDTYPE", "BATTLEGROUND_TRINKET"), ("BACON_TRINKET", "1")];
    ent(&mut g, 392, "BG30_Trinket_1st", &[("CONTROLLER", "2"), ("ZONE", "PLAY"), ("CARDTYPE", "BATTLEGROUND_TRINKET")]);
    ent(&mut g, 3214, "BG30_MagicItem_411", &[trinket.as_slice(), &[("CREATOR", "392")]].concat()); // Hoggy Bank
    ent(&mut g, 3215, "BG36_MagicItem_202", &[trinket.as_slice(), &[("CREATOR", "392"), ("COST", "2")]].concat()); // pivot
    ent(&mut g, 5000, "BG32_MagicItem_366", &[trinket.as_slice(), &[("CREATOR", "0")]].concat()); // seen in combat
    let offer: Vec<(String, i64)> = g.trinket_offer().iter().map(|(e, c)| (e.name(), *c)).collect();
    assert_eq!(offer, [("Hoggy Bank".to_string(), 1), ("Warcry Totem".to_string(), 2)]);
    assert!(g.my_trinkets().is_empty());
    g.set_card_id(392, "BG30_MagicItem_411");
    for eid in [3214, 3215] {
        g.set_tag(eid, "ZONE", "REMOVEDFROMGAME");
    }
    assert!(g.trinket_offer().is_empty());
    assert_eq!(g.my_trinkets().iter().map(|e| e.name()).collect::<Vec<_>>(), ["Hoggy Bank"]);
}

#[test]
fn rank_prefers_my_tribe_and_comp_cards() {
    let k = know();
    let quil = vec![card("Tusked Camper"), card("Snare Trapper"), card("Gilded Gargoyle")];
    let offer = vec![
        (trinket(&k, "Hoggy Bank", "lesser"), 0),
        (trinket(&k, "Ornate Clock", "lesser"), 0),
        (trinket(&k, "Warcry Totem", "lesser"), 2),
    ];
    let picks = rank_offer(&k, &offer, &quil, &[], &[], &[], &[], Some(10));
    // Ornate Clock (no tribe) may win on its stats; the off-tribe Totem must come after my tribe's Bank.
    let at = |name: &str| picks.iter().position(|p| p.card.name == name).unwrap();
    assert!(at("Hoggy Bank") < at("Warcry Totem"));
    let totem = picks.iter().find(|p| p.card.name == "Warcry Totem").unwrap();
    assert!(totem.reasons.iter().any(|r| r.contains("pivot")));
    assert!(totem.reasons.iter().any(|r| r.contains("Dragon/Murloc")), "{:?}", totem.reasons);
    assert!(totem.score > 4.0);
}

#[test]
fn portrait_giving_a_core_card_wins() {
    let k = know();
    let beatboxer = card("Polarizing Beatboxer");
    let comp = Comp {
        id: "mech".into(),
        name: "Mech Test".into(),
        races: vec!["MECHANICAL".into()],
        core: vec![beatboxer],
        addons: vec![],
        difficulty: String::new(),
        when_to_commit: String::new(),
        how_to_play: String::new(),
        stats: Default::default(),
    };
    let offer = vec![(trinket(&k, "Beatboxer Portrait", "greater"), 6), (trinket(&k, "Surveyor Portrait", "greater"), 3)];
    let picks = rank_offer(&k, &offer, &[], &[], &[], &[(&comp, 1.0)], &[], Some(12));
    assert_eq!(picks[0].card.name, "Beatboxer Portrait");
    assert!(picks[0].reasons.iter().any(|r| r.contains("core of Mech Test")));
}

#[test]
fn unaffordable_goes_last() {
    let k = know();
    let offer = vec![(trinket(&k, "Training Certificate", "lesser"), 4), (trinket(&k, "Ornate Clock", "lesser"), 0)];
    let picks = rank_offer(&k, &offer, &[], &[], &[], &[], &[], Some(2));
    assert_eq!(picks[0].card.name, "Ornate Clock");
    assert!(!picks[1].affordable);
}

#[test]
fn hint_before_offer() {
    let k = know();
    assert!(next_offer_hint(&k, 6, &[], &[], None, &[]).is_none());
    assert!(next_offer_hint(&k, 7, &[], &[], None, &[]).is_none());
    let board = vec![card("Tusked Camper"), card("Snare Trapper"), card("Gilded Gargoyle")];
    let h = next_offer_hint(&k, 5, &board, &[], None, &[]).unwrap();
    assert_eq!((h.kind, h.need), ("lesser", 2));
    assert_eq!(h.unlocked, ["QUILBOAR"]);
    assert!(h.close.contains(&"UNDEAD".to_string()), "{:?}", h.close);
    assert!(h.lines.iter().any(|l| l.starts_with("+1 Undead unlocks: Eternal Portrait")), "{:?}", h.lines);
    let h = next_offer_hint(&k, 8, &board, &[], None, &[]).unwrap();
    assert_eq!(h.kind, "greater");
    assert!(h.unlocked.is_empty() && h.close.contains(&"QUILBOAR".to_string()));
}

#[test]
fn race_counts_amalgam_counts_everywhere() {
    let db = cards::test_db();
    let amalgam = db.pool_minions(true).into_iter().find(|c| c.has_race("ALL")).unwrap();
    let n = race_counts(&[card("Tusked Camper"), amalgam]);
    assert_eq!((count(&n, "QUILBOAR"), count(&n, "BEAST")), (2, 1));
}

#[test]
fn trinket_table_is_current_pool() {
    let k = know();
    let db = cards::test_db();
    assert!(k.trinkets.len() >= 250);
    assert!(k.trinkets.keys().all(|i| db.get(i).is_some()));
    let kinds: std::collections::BTreeSet<&str> = k.trinket_list.iter().map(|t| t.kind.as_str()).collect();
    assert_eq!(kinds.into_iter().collect::<Vec<_>>(), ["greater", "lesser"]);
}
