//! error.log in the user folder: what went wrong, with the app version, so a bug report can say more
//! than "it froze". The file is rotated (error.log.1) past 200 KB. Panics are written there too.

use std::io::Write;

use crate::paths;

const LIMIT: u64 = 200_000;

/// Adds one entry to error.log. Never fails (an error while logging is dropped).
pub fn note(context: &str, detail: &str) {
    let path = paths::user_dir().join("error.log");
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
    if std::fs::metadata(&path).map(|m| m.len() > LIMIT).unwrap_or(false) {
        let _ = std::fs::rename(&path, path.with_file_name("error.log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(f, "--- {now} v{} {}\n{context}: {detail}\n", crate::VERSION, std::env::consts::OS);
    }
}

/// Writes panics to error.log (the thread that panicked is restarted by its supervisor).
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        note("panic", &format!("{info}\n{bt}"));
        default(info);
    }));
}
