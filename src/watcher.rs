//! Follows Power.log live (like `tail -f`).
//!
//! - Hearthstone creates a `Logs\Hearthstone_<date>` folder each time it starts and writes Power.log there.
//! - `find_power_log()` returns the Power.log of the newest folder.
//! - The Logs folder is found by `hslogs::find_logs_dir()`, again every `RESCAN` while it is missing;
//!   `status` says what is missing.
//! - `LogWatcher::poll()` reads only the bytes added since the last call and feeds whole lines to the
//!   parser. When the game restarts (new folder) or the file is recreated smaller, it starts over.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::powerlog::PowerLogParser;

/// Time between two searches of the folder (not 4 times a second).
pub const RESCAN: Duration = Duration::from_secs(5);
/// Read by 1 MB blocks: a big Power.log does not take all the memory at once.
pub const CHUNK: usize = 1 << 20;
/// First line of each game in Power.log (the DebugPrintGame header lines come right after it).
pub const GAME_START: &[u8] = b"GameState.DebugPrintPower() - CREATE_GAME";

/// Byte offset of the start of the line holding the last `GAME_START` (0 if none). Reads backwards by
/// blocks: a Power.log of a long session can hold dozens of games and hundreds of MB.
pub fn last_game_start(f: &mut std::fs::File, size: u64) -> std::io::Result<u64> {
    let overlap = GAME_START.len() as u64;
    let mut end = size;
    let mut block = vec![0u8; CHUNK];
    while end > 0 {
        let start = end.saturating_sub(CHUNK as u64);
        let len = (end - start) as usize;
        f.seek(SeekFrom::Start(start))?;
        f.read_exact(&mut block[..len])?;
        let hit = block[..len].windows(GAME_START.len()).rposition(|w| w == GAME_START);
        if let Some(i) = hit {
            let at = start + i as u64;
            // Back to the start of that line.
            let line_start = block[..i].iter().rposition(|&b| b == b'\n').map(|j| start + j as u64 + 1);
            return match line_start {
                Some(p) => Ok(p),
                None => line_start_before(f, at),
            };
        }
        if start == 0 {
            break;
        }
        end = start + overlap.min(start); // keep a marker cut between two blocks
    }
    Ok(0)
}

fn line_start_before(f: &mut std::fs::File, at: u64) -> std::io::Result<u64> {
    let start = at.saturating_sub(4096);
    let mut buf = vec![0u8; (at - start) as usize];
    f.seek(SeekFrom::Start(start))?;
    f.read_exact(&mut buf)?;
    Ok(buf.iter().rposition(|&b| b == b'\n').map(|j| start + j as u64 + 1).unwrap_or(start))
}

/// Power.log of the latest game start, if any.
pub fn find_power_log(logs_dir: &Path) -> Option<PathBuf> {
    let mut folders: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(logs_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("Hearthstone_"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    folders.sort();
    folders.into_iter().rev().map(|(_, f)| f.join("Power.log")).find(|log| log.is_file())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Searching,
    NoFolder,
    NoLog,
    Ok,
}

pub struct LogWatcher {
    fixed_dir: Option<PathBuf>,
    pub logs_dir: Option<PathBuf>,
    pub status: Status,
    last_scan: Option<Instant>,
    pub parser: PowerLogParser,
    /// The file followed.
    pub path: Option<PathBuf>,
    /// Bytes already read in the file.
    pos: u64,
    /// End of a line the game has not finished yet (or cut between two blocks).
    partial: Vec<u8>,
    find_dir: Box<dyn Fn() -> Option<PathBuf> + Send>,
}

impl LogWatcher {
    /// `logs_dir` None: found alone (hslogs), also when the game is installed elsewhere or started later.
    pub fn new(logs_dir: Option<PathBuf>) -> LogWatcher {
        LogWatcher::with_finder(logs_dir, Box::new(crate::hslogs::find_logs_dir))
    }

    pub fn with_finder(logs_dir: Option<PathBuf>, find_dir: Box<dyn Fn() -> Option<PathBuf> + Send>) -> LogWatcher {
        LogWatcher {
            logs_dir: logs_dir.clone(),
            fixed_dir: logs_dir,
            status: Status::Searching,
            last_scan: None,
            parser: PowerLogParser::new(false),
            path: None,
            pos: 0,
            partial: Vec::new(),
            find_dir,
        }
    }

    /// Power.log to follow. The folder and the file are searched only every RESCAN.
    fn locate(&mut self, now: Instant) -> Option<PathBuf> {
        if self.last_scan.is_some_and(|t| now.duration_since(t) < RESCAN) {
            return self.path.clone();
        }
        self.last_scan = Some(now);
        if self.fixed_dir.is_none() {
            self.logs_dir = (self.find_dir)();
        }
        let Some(dir) = &self.logs_dir else {
            self.status = Status::NoFolder;
            return None;
        };
        let path = find_power_log(dir);
        self.status = if path.is_some() { Status::Ok } else { Status::NoLog };
        path
    }

    /// Reads new lines (1 MB at most per call). True if something was read.
    pub fn poll(&mut self) -> bool {
        self.poll_at(Instant::now())
    }

    pub fn poll_at(&mut self, now: Instant) -> bool {
        let Some(path) = self.locate(now) else { return false };
        let data = match self.read_new(&path) {
            Ok(d) => d,
            Err(_) => {
                self.last_scan = None; // deleted meanwhile: search again
                return false;
            }
        };
        if data.is_empty() {
            return false;
        }
        self.partial.extend_from_slice(&data);
        // Split on bytes: a character cut in two between blocks stays whole.
        let end = match self.partial.iter().rposition(|&b| b == b'\n') {
            Some(i) => i + 1,
            None => return true,
        };
        let done: Vec<u8> = self.partial.drain(..end).collect();
        for line in done[..done.len() - 1].split(|&b| b == b'\n') {
            self.parser.feed_line(&String::from_utf8_lossy(line));
        }
        true
    }

    fn read_new(&mut self, path: &Path) -> std::io::Result<Vec<u8>> {
        let size = std::fs::metadata(path)?.len();
        let mut f = std::fs::File::open(path)?;
        if self.path.as_deref() != Some(path) || size < self.pos {
            // New file (game restarted) or recreated smaller: read again from the start. The first file
            // opened starts at its last game: older games of the session are not replayed.
            let first = self.path.is_none();
            self.path = Some(path.to_path_buf());
            self.pos = if first { last_game_start(&mut f, size)? } else { 0 };
            self.partial.clear();
            self.parser = PowerLogParser::new(false);
        }
        f.seek(SeekFrom::Start(self.pos))?;
        let mut data = Vec::with_capacity(CHUNK.min((size - self.pos) as usize));
        f.take(CHUNK as u64).read_to_end(&mut data)?;
        self.pos += data.len() as u64;
        Ok(data)
    }

    /// True once the file is read up to its current end.
    pub fn caught_up(&self) -> bool {
        self.path.as_ref().and_then(|p| std::fs::metadata(p).ok()).is_some_and(|m| m.len() <= self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("owl-watch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn statuses_and_newest_log() {
        let logs = dir();
        let none: Box<dyn Fn() -> Option<PathBuf> + Send> = Box::new(|| None);
        let mut w = LogWatcher::with_finder(None, none);
        assert!(!w.poll());
        assert_eq!(w.status, Status::NoFolder);

        let mut w = LogWatcher::new(Some(logs.clone()));
        assert!(!w.poll());
        assert_eq!(w.status, Status::NoLog);

        let old = logs.join("Hearthstone_2026_01_01");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("Power.log"), b"").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let new = logs.join("Hearthstone_2026_01_02");
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("Power.log"), b"").unwrap();
        assert_eq!(find_power_log(&logs), Some(new.join("Power.log")));
        std::fs::remove_dir_all(logs).ok();
    }

    #[test]
    fn starts_at_the_last_game() {
        let logs = dir();
        let folder = logs.join("Hearthstone_x");
        std::fs::create_dir_all(&folder).unwrap();
        let log = folder.join("Power.log");
        let game = "D 00:00:01.0000000 GameState.DebugPrintPower() - CREATE_GAME\n";
        let filler = "D 00:00:02.0000000 GameState.DebugPrintPower() -     tag=ZONE value=PLAY\n".repeat(30_000);
        std::fs::write(&log, format!("{game}{filler}{game}{filler}")).unwrap();
        let size = std::fs::metadata(&log).unwrap().len();
        let mut f = std::fs::File::open(&log).unwrap();
        let at = last_game_start(&mut f, size).unwrap();
        assert_eq!(at, (game.len() + filler.len()) as u64);
        // Marker cut between two blocks.
        for cut in [CHUNK - 10, CHUNK - 60, CHUNK + 5] {
            let pad = "x".repeat(cut - game.len() + 30);
            std::fs::write(&log, format!("{game}{pad}\n{game}{}", "y\n".repeat(CHUNK / 2 - 3))).unwrap();
            let size = std::fs::metadata(&log).unwrap().len();
            let mut f = std::fs::File::open(&log).unwrap();
            assert_eq!(last_game_start(&mut f, size).unwrap(), (game.len() + pad.len() + 1) as u64, "cut {cut}");
        }
        std::fs::write(&log, "no game here\n").unwrap();
        let mut f = std::fs::File::open(&log).unwrap();
        assert_eq!(last_game_start(&mut f, 13).unwrap(), 0);
        std::fs::remove_dir_all(logs).ok();
    }

    #[test]
    fn reads_whole_lines_only_and_restarts_on_truncation() {
        let logs = dir();
        let folder = logs.join("Hearthstone_x");
        std::fs::create_dir_all(&folder).unwrap();
        let log = folder.join("Power.log");
        let line = "D 00:00:01.0000000 GameState.DebugPrintPower() - CREATE_GAME\n";
        std::fs::write(&log, &line.as_bytes()[..20]).unwrap();
        let mut w = LogWatcher::new(Some(logs.clone()));
        let t = Instant::now();
        assert!(w.poll_at(t));
        assert!(w.parser.game().is_none(), "half a line is kept for later");
        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        f.write_all(&line.as_bytes()[20..]).unwrap();
        drop(f);
        assert!(w.poll_at(t));
        assert!(w.parser.game().is_some());
        assert!(!w.poll_at(t));
        assert!(w.caught_up());

        std::fs::write(&log, b"x\n").unwrap(); // recreated, smaller
        assert!(w.poll_at(t));
        assert!(w.parser.game().is_none());
        std::fs::remove_dir_all(logs).ok();
    }
}
