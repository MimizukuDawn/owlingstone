//! Owlingstone.exe: the window over the game (no argument), or a console command.
//!
//! - `replay <file.log>`: re-reads an old log and prints the state at the start of each recruit phase.
//! - `live`: follows the game going on and prints the state at each change.
//! - `history [--import [folder or file]]`: my stats (average place by comp, Deity, hero, each rule);
//!   `--import` first adds the games of old Power.log files (default: the game's Logs folder).
//! - `snapshot <file.log> [--turn N]`: what the window would show at that turn, as text (tests).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use owlingstone::powerlog::PowerLogParser;
use owlingstone::{cards, errlog, history, hslogs, knowledge, snapshot, watcher};

const USAGE: &str = "Usage: Owlingstone [replay <log> | live | history [--import [path]] | snapshot <log> [--turn N]]";

fn main() {
    errlog::install_panic_hook();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        None | Some("ui") => run_ui(),
        Some("replay") => args.get(1).map(|p| replay(&PathBuf::from(p))).unwrap_or_else(usage),
        Some("live") => live(),
        Some("history") => history(&args[1..]),
        Some("snapshot") => match args.get(1) {
            Some(p) => {
                let turn = args.iter().position(|a| a == "--turn").and_then(|i| args.get(i + 1)).and_then(|t| t.parse().ok());
                print_snapshot(&PathBuf::from(p), turn)
            }
            None => usage(),
        },
        Some("-V" | "--version") => {
            println!("Owlingstone {}", owlingstone::VERSION);
            0
        }
        Some(_) => usage(),
    };
    std::process::exit(code);
}

fn usage() -> i32 {
    eprintln!("{USAGE}");
    2
}

/// The window comes with the next step of the Rust version.
fn run_ui() -> i32 {
    eprintln!("The window is not built into this version yet.\n{USAGE}");
    2
}

fn replay(path: &std::path::Path) -> i32 {
    let text = match std::fs::read(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            return 1;
        }
    };
    let mut parser = PowerLogParser::new(true);
    let mut last: Option<i64> = None;
    for line in String::from_utf8_lossy(&text).lines() {
        parser.feed_line(line);
        let Some(g) = parser.game() else { continue };
        // Printed when the shop of the turn just appeared (start of the recruit phase).
        let turn = g.turn();
        if last.is_some_and(|t| t == turn) || !g.is_battlegrounds() {
            continue;
        }
        if !g.shop().is_empty() {
            last = Some(turn);
            println!("{}", g.summary());
        }
    }
    for (i, g) in parser.games().iter().enumerate().filter(|(_, g)| g.is_battlegrounds()) {
        println!(
            "--- game {} over: place {:?}, hero {}, tribes {:?}",
            i + 1,
            g.my_placement(),
            g.my_hero().map(|h| h.name()).unwrap_or_else(|| "?".into()),
            g.available_races()
        );
    }
    0
}

fn live() -> i32 {
    println!("Waiting for Hearthstone... (Ctrl+C to stop)");
    let mut w = watcher::LogWatcher::new(None);
    let mut last = None;
    loop {
        if w.poll() && w.caught_up() {
            if let Some(g) = w.parser.game().filter(|g| g.is_battlegrounds()) {
                let s = g.summary();
                if last.as_ref() != Some(&s) {
                    println!("{s}");
                    last = Some(s);
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

fn history(args: &[String]) -> i32 {
    let k = knowledge::get();
    let path = history::history_path();
    if let Some(i) = args.iter().position(|a| a == "--import") {
        let src = match args.get(i + 1) {
            Some(p) => PathBuf::from(p),
            None => match hslogs::find_logs_dir() {
                Some(d) => d,
                None => {
                    eprintln!("Hearthstone's Logs folder not found: give it, e.g. history --import \"D:\\Hearthstone\\Logs\"");
                    return 1;
                }
            },
        };
        let paths = log_files(&src);
        match history::add_records(&history::import_logs(&paths, &k), &path) {
            Ok(added) => println!("{} files read, {added} new games in the history.\n", paths.len()),
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                return 1;
            }
        }
    }
    println!("{}", history::format_report(&history::load_history(&path), &k.rules));
    0
}

/// `Hearthstone_*/Power*.log` in a Logs folder (sorted), or the file itself.
fn log_files(src: &std::path::Path) -> Vec<PathBuf> {
    if !src.is_dir() {
        return vec![src.to_path_buf()];
    }
    let mut found = Vec::new();
    for folder in std::fs::read_dir(src).into_iter().flatten().flatten() {
        if !folder.file_name().to_string_lossy().starts_with("Hearthstone_") {
            continue;
        }
        for f in std::fs::read_dir(folder.path()).into_iter().flatten().flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if name.starts_with("Power") && name.ends_with(".log") {
                found.push(f.path());
            }
        }
    }
    found.sort();
    found
}

fn print_snapshot(path: &std::path::Path, turn: Option<i64>) -> i32 {
    match snapshot::replay_snapshot(path, turn) {
        Ok(s) => {
            println!("{s:#?}");
            0
        }
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            1
        }
    }
}

#[allow(dead_code)]
fn start_reader(sink: snapshot::Sink) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    cards::refresh_in_background(false);
    std::thread::Builder::new().name("log reader".into()).spawn(move || snapshot::run_live(sink, s)).ok();
    stop
}
