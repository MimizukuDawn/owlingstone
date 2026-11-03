//! Owlingstone.exe: the window over the game (no argument), or a console command.
//!
//! - `replay <file.log>`: re-reads an old log and prints the state at the start of each recruit phase.
//! - `live`: follows the game going on and prints the state at each change.
//! - `history [--import [folder or file]]`: my stats (average place by comp, Deity, hero, each rule);
//!   `--import` first adds the games of old Power.log files (default: the game's Logs folder).
//! - `snapshot <file.log> [--turn N]`: what the window would show at that turn, as text (tests).

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;

use std::path::PathBuf;

use owlingstone::powerlog::PowerLogParser;
use owlingstone::{errlog, history, hslogs, knowledge, snapshot, watcher};

const USAGE: &str = "Usage: Owlingstone [ui [--replay <log> [--turn N]] | replay <log> | live | history [--import [path]] | snapshot <log> [--turn N]]";

fn main() {
    errlog::install_panic_hook();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !matches!(args.first().map(String::as_str), None | Some("ui")) {
        attach_console();
    }
    let code = match args.first().map(String::as_str) {
        None | Some("ui") => run_ui(&args),
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

/// The exe is a window app, so a console command run from cmd / PowerShell prints into that console.
#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: plain Win32 call; failure (no parent console, or output already redirected) is harmless.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(not(windows))]
fn attach_console() {}

fn usage() -> i32 {
    eprintln!("{USAGE}");
    2
}

/// The window over the game. `ui --replay <log> [--turn N]`: an old log instead of the live game.
fn run_ui(args: &[String]) -> i32 {
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1));
    let replay = value("--replay").map(|p| (PathBuf::from(p), value("--turn").and_then(|t| t.parse().ok())));
    let smoke = std::env::var_os("OWLINGSTONE_SMOKE").is_some_and(|v| v == "1");
    app::run(app::Options { replay, smoke })
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
