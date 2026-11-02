//! Where Hearthstone writes its logs, and making sure it writes Power.log at all.
//!
//! - `logs_dir_candidates()` lists, best first: the folder chosen by the player (tray menu "Choose
//!   Hearthstone folder…", saved as `hearthstone_dir` in settings.json), the folder of the running
//!   Hearthstone.exe, the install folder Battle.net writes in the Windows registry, then the usual folders.
//! - `find_logs_dir()` returns the first one that exists: Hearthstone installed on D:\Games works too.
//! - `ensure_log_config()` adds the [Power] section to `%LOCALAPPDATA%\Blizzard\Hearthstone\log.config`
//!   if it is missing, like other trackers do: without it the game writes no Power.log. Hearthstone reads
//!   that file when it starts, so a game already running must be restarted once.

use std::path::{Path, PathBuf};

pub const DEFAULT_INSTALLS: [&str; 2] = [r"C:\Program Files (x86)\Hearthstone", r"C:\Program Files\Hearthstone"];
pub const UNINSTALL_KEYS: [&str; 2] = [
    r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Hearthstone",
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Hearthstone",
];
pub const POWER_SECTION: &str =
    "[Power]\nLogLevel=1\nFilePrinting=true\nConsolePrinting=false\nScreenPrinting=false\nVerbose=true\n";

/// The player may pick the install folder or its Logs folder: both are fine.
pub fn as_logs_dir(folder: &Path) -> PathBuf {
    let is_logs = folder.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("logs"));
    if is_logs { folder.to_path_buf() } else { folder.join("Logs") }
}

/// Logs folders to try, best first, without duplicates.
pub fn logs_dir_candidates(chosen: Option<&Path>, running: Option<PathBuf>, registry: Option<PathBuf>) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let installs = chosen
        .map(Path::to_path_buf)
        .into_iter()
        .chain(running)
        .chain(registry)
        .chain(DEFAULT_INSTALLS.iter().map(PathBuf::from));
    for install in installs {
        let logs = as_logs_dir(&install);
        if !found.contains(&logs) {
            found.push(logs);
        }
    }
    found
}

/// First candidate whose Logs folder (or Hearthstone install) exists.
pub fn pick_logs_dir(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates
        .iter()
        .find(|logs| logs.is_dir() || logs.parent().is_some_and(|p| p.join("Hearthstone.exe").exists()))
        .cloned()
}

/// The Logs folder of this computer's Hearthstone (chosen folder from settings.json first).
pub fn find_logs_dir() -> Option<PathBuf> {
    let chosen = crate::settings::Settings::load().hearthstone_dir().map(PathBuf::from);
    pick_logs_dir(&logs_dir_candidates(chosen.as_deref(), running_install(), registry_install()))
}

pub fn log_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty())?;
    Some(PathBuf::from(base).join("Blizzard").join("Hearthstone").join("log.config"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogConfig {
    /// Power logging was already on.
    Ok,
    /// We just turned it on: a running game must be restarted once.
    Written,
}

/// Turns Power logging on in `path` (default: the game's log.config). None if we could not.
pub fn ensure_log_config(path: Option<&Path>) -> Option<LogConfig> {
    let path = match path {
        Some(p) => p.to_path_buf(),
        None => log_config_path()?,
    };
    let text = match std::fs::read(&path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return None,
    };
    if text.to_lowercase().contains("[power]") {
        return Some(LogConfig::Ok);
    }
    let lead = if !text.is_empty() && !text.ends_with('\n') { "\n" } else { "" };
    let add = format!("{lead}{POWER_SECTION}").replace('\n', "\r\n");
    std::fs::create_dir_all(path.parent()?).ok()?;
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()?;
    f.write_all(add.as_bytes()).ok()?;
    Some(LogConfig::Written)
}

/// Folder of the running Hearthstone.exe.
#[cfg(windows)]
pub fn running_install() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};
    let class = wide("UnityWndClass");
    let title = wide("Hearthstone");
    // SAFETY: plain Win32 calls with valid NUL-terminated strings and buffers that outlive them; the
    // process handle is closed before returning.
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), title.as_ptr());
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut size);
        CloseHandle(handle);
        if ok == 0 || size == 0 {
            return None;
        }
        let exe = PathBuf::from(std::ffi::OsString::from_wide(&buf[..size as usize]));
        exe.parent().map(Path::to_path_buf)
    }
}

#[cfg(not(windows))]
pub fn running_install() -> Option<PathBuf> {
    None
}

/// InstallLocation that Battle.net writes in the registry.
#[cfg(windows)]
pub fn registry_install() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    let name = wide("InstallLocation");
    for key in UNINSTALL_KEYS {
        let key = wide(key);
        let mut buf = vec![0u16; 1024];
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: the key and value names are NUL-terminated; `bytes` holds the buffer's size in bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != 0 {
            continue;
        }
        let len = (bytes as usize / 2).min(buf.len());
        let value: Vec<u16> = buf[..len].iter().copied().take_while(|&c| c != 0).collect();
        if !value.is_empty() {
            return Some(PathBuf::from(std::ffi::OsString::from_wide(&value)));
        }
    }
    None
}

#[cfg(not(windows))]
pub fn registry_install() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_folder_or_install_folder() {
        assert_eq!(as_logs_dir(Path::new("/g/Hearthstone")), Path::new("/g/Hearthstone/Logs"));
        assert_eq!(as_logs_dir(Path::new("/g/Hearthstone/logs")), Path::new("/g/Hearthstone/logs"));
    }

    #[test]
    fn candidates_in_order_without_duplicates() {
        let c = logs_dir_candidates(
            Some(Path::new("/d/HS/Logs")),
            Some(PathBuf::from("/d/HS")),
            Some(PathBuf::from("/e/HS")),
        );
        assert_eq!(c[0], Path::new("/d/HS/Logs"));
        assert_eq!(c[1], Path::new("/e/HS/Logs"));
        assert_eq!(c.len(), 2 + DEFAULT_INSTALLS.len());
    }

    #[test]
    fn picks_first_existing() {
        let dir = std::env::temp_dir().join(format!("owl-hslogs-{}", uuid::Uuid::new_v4()));
        let install = dir.join("HS");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("Hearthstone.exe"), b"").unwrap();
        let c = logs_dir_candidates(Some(&dir.join("missing")), Some(install.clone()), None);
        assert_eq!(pick_logs_dir(&c), Some(install.join("Logs")));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn log_config_is_appended_once() {
        let dir = std::env::temp_dir().join(format!("owl-logcfg-{}", uuid::Uuid::new_v4()));
        let path = dir.join("Blizzard").join("log.config");
        assert_eq!(ensure_log_config(Some(&path)), Some(LogConfig::Written));
        assert_eq!(ensure_log_config(Some(&path)), Some(LogConfig::Ok));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("[Power]\r\nLogLevel=1\r\n"));

        std::fs::write(&path, "[Zone]\nLogLevel=1").unwrap();
        assert_eq!(ensure_log_config(Some(&path)), Some(LogConfig::Written));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("[Zone]\nLogLevel=1\r\n{}", POWER_SECTION.replace('\n', "\r\n")));
        std::fs::remove_dir_all(dir).ok();
    }
}
