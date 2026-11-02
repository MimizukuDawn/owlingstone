//! The live reader on a real log (private log in testdata/, skipped when missing): a session log with
//! two games is read from its last game only, shows one finished-game view, writes the game to the
//! history once.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use owlingstone::snapshot::{self, Snapshot};

#[test]
fn live_reader_reads_the_last_game_once() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/bg_game1.log");
    if !src.exists() {
        eprintln!("testdata/bg_game1.log missing: skipped");
        return;
    }
    let home = std::env::temp_dir().join(format!("owl-live-{}", uuid::Uuid::new_v4()));
    let logs = home.join("HS").join("Logs");
    let folder = logs.join("Hearthstone_2026_10_30_20_00_00");
    std::fs::create_dir_all(&folder).unwrap();
    let game = std::fs::read(&src).unwrap();
    // The same game twice in one session: only the last one is read.
    let mut session = game.clone();
    session.extend_from_slice(&game);
    std::fs::write(folder.join("Power.log"), &session).unwrap();
    std::fs::write(home.join("settings.json"), serde_json::json!({"hearthstone_dir": home.join("HS")}).to_string()).unwrap();
    unsafe {
        std::env::set_var("OWLINGSTONE_HOME", &home);
        std::env::set_var("OWLINGSTONE_NO_CARD_DOWNLOAD", "1");
    }

    let (tx, rx) = mpsc::channel::<Arc<Snapshot>>();
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let reader = std::thread::spawn(move || snapshot::run_live(Box::new(move |snap| drop(tx.send(snap))), s));
    let start = Instant::now();
    let mut seen = Vec::new();
    while start.elapsed() < Duration::from_secs(60) {
        if let Ok(snap) = rx.recv_timeout(Duration::from_millis(200)) {
            seen.push(snap);
        }
        if seen.last().is_some_and(|s| s.over) && home.join("history.jsonl").exists() {
            std::thread::sleep(Duration::from_millis(600)); // a few more polls: nothing new must come
            seen.extend(rx.try_iter());
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    reader.join().unwrap();

    assert_eq!(seen.len(), 1, "one view once caught up, no replay of the game: {:?}", seen.iter().map(|s| (s.in_game, s.turn)).collect::<Vec<_>>());
    let last = &seen[0];
    assert!(!last.in_game && last.over);
    assert_eq!(last.placement, Some(6));
    let history = std::fs::read_to_string(home.join("history.jsonl")).unwrap();
    assert_eq!(history.lines().count(), 1);
    assert!(history.contains("\"date\":\"2026-10-30\""), "{history}");
    std::fs::remove_dir_all(home).ok();
}
