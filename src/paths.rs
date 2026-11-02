//! Where files are.
//!
//! - User folder (`user_dir`): the player's own files, never touched by an update: history, settings,
//!   card images, cards.json, learned tribes (subsets.json), error.log... On Windows
//!   `%APPDATA%\Owlingstone`, the same folder as the Python versions, so nothing is lost on the switch.
//! - Data folder (`data_dir`): the game knowledge in `data/*.yaml`. A copy is built into the exe; when
//!   the exe runs from a source checkout (or OWLINGSTONE_DATA points somewhere), the files there are
//!   used instead and reloaded when they change, so editing rules.yaml applies without a rebuild.

use std::path::PathBuf;

/// The player's folder. `OWLINGSTONE_HOME` (or the older `BGTRACKER_HOME`) moves it, for tests.
pub fn user_dir() -> PathBuf {
    for var in ["OWLINGSTONE_HOME", "BGTRACKER_HOME"] {
        if let Some(v) = std::env::var_os(var).filter(|v| !v.is_empty()) {
            return PathBuf::from(v);
        }
    }
    if let Some(appdata) = std::env::var_os("APPDATA").filter(|v| !v.is_empty()) {
        return PathBuf::from(appdata).join("Owlingstone");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".owlingstone")
}

/// A `data/` folder to read instead of the built-in copy, if there is one.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("OWLINGSTONE_DATA").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(v));
    }
    let exe = std::env::current_exe().ok()?;
    // Built in a checkout (target/release/Owlingstone.exe): use the checkout's data/.
    for folder in exe.ancestors().skip(1) {
        if folder.join("data").join("comps.yaml").is_file() && folder.join("Cargo.toml").is_file() {
            return Some(folder.join("data"));
        }
    }
    None
}
