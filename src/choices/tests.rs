use super::*;
use crate::knowledge::HeroStat;
use crate::powerlog::PowerLogParser;
use crate::rules::Ctx;
use crate::{cards, data};

fn know() -> Knowledge {
    Knowledge::build(&cards::test_db(), &|n| data::read(n))
}

fn card(name: &str) -> Arc<Card> {
    let db = cards::test_db();
    let pool: Vec<Arc<Card>> = db.pool_minions(false).into_iter().chain(db.pool_spells(false)).collect();
    pool.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{name}")).clone()
}

#[test]
fn chances_sum_to_100_and_follow_the_gap() {
    let c = chances(&[1.0, 1.0, 1.0], 1.5);
    assert!(c.iter().all(|x| (x - 100.0 / 3.0).abs() < 1e-9));
    let c = chances(&[2.5, 1.0], 1.5);
    assert!((c.iter().sum::<f64>() - 100.0).abs() < 1e-9);
    assert!((c[0] / c[1] - std::f64::consts::E).abs() < 1e-9);
    assert!(chances(&[], 1.0).is_empty());
}

#[test]
fn heroes_in_screen_order_best_marked() {
    let hero = |name: &str, id: &str, avg: Option<f64>| HeroOption {
        name: name.into(),
        card_id: id.into(),
        stat: avg.map(|a| HeroStat { avg: a, games: 1000.0 }),
        best_comps: vec![],
        note: String::new(),
    };
    // advice.rs order (best first) differs from the screen.
    let ranked = vec![hero("B", "b", Some(3.6)), hero("A", "a", Some(4.1)), hero("X", "x", None)];
    let screen: Vec<String> = ["a", "x", "b"].iter().map(|s| s.to_string()).collect();
    let c = for_heroes(&ranked, &screen).unwrap();
    assert_eq!(c.options.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(), ["A", "X", "B"]);
    assert!(c.options[2].best && !c.options[0].best);
    assert!(c.options[2].chance > 60.0, "{}", describe(&c));
    assert_eq!(c.options[1].score, "no stats");
}

#[test]
fn discover_prefers_the_core_card_of_my_comp() {
    let k = know();
    let lobby: Vec<String> = ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"].iter().map(|s| s.to_string()).collect();
    let ctx = Ctx { races: Some(lobby), ..Default::default() };
    let board = vec![card("Spark Snapper"), card("Annoy-o-Module")];
    let targets = advice::target_comps(&k, &board, &[], &ctx);
    let options = vec![card("Wrath Weaver"), card("Auto Reveille"), card("Glambot")];
    let c = for_discover(&k, Some("Triple Reward"), &options, &board, &[], &board, &targets, None, 6, 3).unwrap();
    assert_eq!(c.title, "Discover · Triple Reward");
    let gift = for_discover(&k, Some("Battlegrounds Dark Gift [DNT]"), &options, &board, &[], &board, &targets, None, 6, 3).unwrap();
    assert_eq!(gift.title, "Discover · Dark Gift");
    let best = c.options.iter().find(|o| o.best).unwrap();
    assert_eq!(best.name, "Auto Reveille", "{}", describe(&c));
    assert!(best.reasons.iter().any(|r| r.starts_with("core ")), "{:?}", best.reasons);
    assert!((c.options.iter().map(|o| o.chance).sum::<f64>() - 100.0).abs() < 1e-6);
}

fn lines(body: &[&str]) -> Vec<String> {
    body.iter().map(|b| format!("D 21:00:00.0000000 GameState.{b}")).collect()
}

/// A Discover as Power.log writes it: options created in SETASIDE, then the choice, then the pick.
fn discover_log() -> Vec<String> {
    lines(&[
        "DebugPrintPower() - CREATE_GAME",
        "DebugPrintPower() -     GameEntity EntityID=1",
        "DebugPrintPower() -         tag=TURN value=11",
        "DebugPrintPower() -     Player EntityID=2 PlayerID=5 GameAccountId=[hi=144115 lo=2802]",
        "DebugPrintPower() -     Player EntityID=3 PlayerID=13 GameAccountId=[hi=0 lo=0]",
        "DebugPrintGame() - GameType=GT_BATTLEGROUNDS",
        "DebugPrintGame() - PlayerID=5, PlayerName=Me#1234",
        "DebugPrintGame() - PlayerID=13, PlayerName=Bob",
        "DebugPrintPower() -     FULL_ENTITY - Creating ID=300 CardID=BG_GVG_048",
        "DebugPrintPower() -         tag=CONTROLLER value=5",
        "DebugPrintPower() -         tag=CARDTYPE value=MINION",
        "DebugPrintPower() -         tag=ZONE value=SETASIDE",
        "DebugPrintPower() -     FULL_ENTITY - Creating ID=301 CardID=BG26_ComputedPick",
        "DebugPrintPower() -         tag=CONTROLLER value=5",
        "DebugPrintPower() -         tag=CARDTYPE value=MINION",
        "DebugPrintPower() -         tag=ZONE value=SETASIDE",
        "DebugPrintPower() -     FULL_ENTITY - Creating ID=302 CardID=",
        "DebugPrintPower() -         tag=CONTROLLER value=5",
        "DebugPrintPower() -         tag=CARDTYPE value=MINION",
        "DebugPrintPower() -         tag=ZONE value=SETASIDE",
        "DebugPrintEntityChoices() - id=9 Player=Me#1234 TaskList=77 ChoiceType=GENERAL CountMin=1 CountMax=1",
        "DebugPrintEntityChoices() -   Source=[entityName=Triple Reward id=250 zone=PLAY zonePos=0 cardId=TB_BaconShop_Triples_01 player=5]",
        "DebugPrintEntityChoices() -   Entities[0]=[entityName=Metaltooth Leaper id=300 zone=SETASIDE zonePos=0 cardId=BG_GVG_048 player=5]",
        "DebugPrintEntityChoices() -   Entities[1]=[entityName=Pick id=301 zone=SETASIDE zonePos=0 cardId=BG26_ComputedPick player=5]",
        "DebugPrintEntityChoices() -   Entities[2]=[entityName=Hidden id=302 zone=SETASIDE zonePos=0 cardId=BG_CFM_315 player=5]",
    ])
}

#[test]
fn power_log_choice_is_read_in_screen_order() {
    let mut p = PowerLogParser::new(true);
    let log = discover_log();
    p.feed_lines(log.iter().map(String::as_str));
    let g = p.game().unwrap();
    let (c, options) = g.open_choice().expect("open choice");
    assert_eq!(c.id, 9);
    assert_eq!(c.choice_type, "GENERAL");
    assert_eq!(c.turn, 6);
    assert_eq!(options.iter().map(|e| e.id).collect::<Vec<_>>(), [300, 301, 302]);
    // The card the log hid in FULL_ENTITY but named in the choice line.
    assert_eq!(options[2].card_id, "BG_CFM_315");
    assert_eq!(c.source, None, "source entity never created: unknown");
}

#[test]
fn choice_closes_when_made_or_taken() {
    let mut log = discover_log();
    log.extend(lines(&[
        "DebugPrintEntitiesChosen() - id=9 Player=Me#1234 EntitiesCount=1",
        "DebugPrintEntitiesChosen() -   Entities[0]=[entityName=Metaltooth Leaper id=300 zone=SETASIDE zonePos=0 cardId=BG_GVG_048 player=5]",
    ]));
    let mut p = PowerLogParser::new(true);
    p.feed_lines(log.iter().map(String::as_str));
    assert!(p.game().unwrap().open_choice().is_none());

    // No "chosen" line, but the pick went to my hand: closed too.
    let mut log = discover_log();
    log.extend(lines(&["DebugPrintPower() -     TAG_CHANGE Entity=300 tag=ZONE value=HAND"]));
    let mut p = PowerLogParser::new(true);
    p.feed_lines(log.iter().map(String::as_str));
    assert!(p.game().unwrap().open_choice().is_none());

    // Next turn: stale.
    let mut log = discover_log();
    log.extend(lines(&["DebugPrintPower() -     TAG_CHANGE Entity=GameEntity tag=TURN value=13"]));
    let mut p = PowerLogParser::new(true);
    p.feed_lines(log.iter().map(String::as_str));
    assert!(p.game().unwrap().open_choice().is_none());
}

#[test]
fn another_players_choice_is_ignored() {
    let mut log = discover_log();
    log.extend(lines(&[
        "DebugPrintGame() - PlayerID=13, PlayerName=Bob",
        "DebugPrintEntityChoices() - id=10 Player=Bob TaskList= ChoiceType=GENERAL CountMin=1 CountMax=1",
        "DebugPrintEntityChoices() -   Entities[0]=[entityName=Metaltooth Leaper id=300 zone=SETASIDE zonePos=0 cardId=BG_GVG_048 player=5]",
    ]));
    let mut p = PowerLogParser::new(true);
    p.feed_lines(log.iter().map(String::as_str));
    assert!(p.game().unwrap().open_choice().is_none());
}
