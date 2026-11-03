//! What the window needs from Windows: where Hearthstone is on screen, which program is in front,
//! whether another tracker runs, one Owlingstone at a time, opening a folder. On other systems (tests,
//! screenshots) these answer "nothing found".

/// A rectangle on screen in physical pixels: (x, y, width, height).
pub type Rect = (i32, i32, i32, i32);

#[cfg(windows)]
mod imp {
    use super::Rect;
    use std::path::{Path, PathBuf};
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, POINT, RECT};
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        CreateMutexW, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, FindWindowW, GetClientRect, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, SW_SHOWNORMAL,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The game area of Hearthstone's window, or None (not running, or minimized).
    pub fn hearthstone_rect() -> Option<Rect> {
        let (class, title) = (wide("UnityWndClass"), wide("Hearthstone"));
        // SAFETY: Win32 calls with NUL-terminated strings and out-structs that live on the stack.
        unsafe {
            let hwnd = FindWindowW(class.as_ptr(), title.as_ptr());
            if hwnd.is_null() || IsIconic(hwnd) != 0 {
                return None;
            }
            let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            let mut origin = POINT { x: 0, y: 0 };
            if GetClientRect(hwnd, &mut rc) == 0 || ClientToScreen(hwnd, &mut origin) == 0 {
                return None;
            }
            (rc.right > 0 && rc.bottom > 0).then_some((origin.x, origin.y, rc.right, rc.bottom))
        }
    }

    /// Where the mouse is on screen (physical pixels).
    pub fn cursor_pos() -> Option<(i32, i32)> {
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: out-struct on the stack.
        (unsafe { GetCursorPos(&mut p) } != 0).then_some((p.x, p.y))
    }

    /// True while the left or right mouse button is down, wherever the mouse is (even over other
    /// programs): the tray menu closes on a click outside it.
    pub fn mouse_down() -> bool {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
        // SAFETY: plain Win32 query, no pointers.
        unsafe { (GetAsyncKeyState(VK_LBUTTON as i32) as u16 & 0x8000) != 0 || (GetAsyncKeyState(VK_RBUTTON as i32) as u16 & 0x8000) != 0 }
    }

    /// File name of the program whose window is in front, in lower case (e.g. "hearthstone.exe").
    pub fn foreground_exe() -> String {
        // SAFETY: as above; the process handle is closed before returning.
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return String::new();
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return String::new();
            }
            let mut buf = [0u16; 512];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut size);
            CloseHandle(handle);
            if ok == 0 {
                return String::new();
            }
            let path = PathBuf::from(String::from_utf16_lossy(&buf[..size as usize]));
            path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
        }
    }

    /// Names (lower case) of the running programs.
    pub fn running_exes() -> Vec<String> {
        let mut names = Vec::new();
        // SAFETY: the snapshot handle is closed; dwSize is set as the API requires.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap.is_null() || snap as isize == -1 {
                return names;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut entry);
            while ok != 0 {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                names.push(String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase());
                ok = Process32NextW(snap, &mut entry);
            }
            CloseHandle(snap);
        }
        names
    }

    /// Held for the whole run: a second Owlingstone sees it and quits.
    pub struct Instance(HANDLE);

    // SAFETY: the handle is only closed on drop, from any thread.
    unsafe impl Send for Instance {}

    impl Drop for Instance {
        fn drop(&mut self) {
            // SAFETY: our own mutex handle.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// One Owlingstone per Windows session. Waits up to `wait` for another one to quit (after an update).
    pub fn single_instance(wait: std::time::Duration) -> Option<Instance> {
        let name = wide("Local\\Owlingstone.SingleInstance");
        let until = std::time::Instant::now() + wait;
        loop {
            // SAFETY: a named mutex; closed by Instance::drop, or right away if another one owns it.
            unsafe {
                let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
                if !h.is_null() && GetLastError() != ERROR_ALREADY_EXISTS {
                    return Some(Instance(h));
                }
                if !h.is_null() {
                    CloseHandle(h);
                }
            }
            if std::time::Instant::now() >= until {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }

    /// Identifies the window in front (changes when the player switches program).
    pub fn foreground_window() -> usize {
        // SAFETY: no arguments; the handle is only compared, never used.
        unsafe { GetForegroundWindow() as usize }
    }

    /// Turns off Windows' fade when our windows show or hide, so the panels appear and vanish at once.
    /// Must run on the thread that made the windows (the window thread).
    pub fn no_fade_on_own_windows() {
        unsafe extern "system" fn each(hwnd: HWND, _: LPARAM) -> BOOL {
            let off: BOOL = 1;
            // SAFETY: a window of this thread, a BOOL that lives for the call.
            unsafe {
                DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED as u32, (&raw const off).cast(), size_of::<BOOL>() as u32);
            }
            1
        }
        // SAFETY: the callback only touches the window handed to it.
        unsafe {
            EnumThreadWindows(GetCurrentThreadId(), Some(each), 0);
        }
    }

    /// Opens a folder in Explorer.
    pub fn open_folder(path: &Path) {
        let (verb, target) = (wide("open"), wide(&path.to_string_lossy()));
        // SAFETY: NUL-terminated strings; no window handle needed.
        unsafe {
            ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Rect;

    pub fn hearthstone_rect() -> Option<Rect> {
        None
    }
    pub fn foreground_exe() -> String {
        String::new()
    }
    pub fn cursor_pos() -> Option<(i32, i32)> {
        None
    }
    pub fn mouse_down() -> bool {
        false
    }
    pub fn running_exes() -> Vec<String> {
        Vec::new()
    }
    pub struct Instance;
    pub fn single_instance(_wait: std::time::Duration) -> Option<Instance> {
        Some(Instance)
    }
    pub fn open_folder(_path: &std::path::Path) {}
    pub fn foreground_window() -> usize {
        0
    }
    pub fn no_fade_on_own_windows() {}
}

pub use imp::*;

/// Other trackers that already show win chances (their overlay sits at the same place).
pub const OTHER_TRACKERS: [&str; 3] = ["hearthstonedecktracker.exe", "firestone.exe", "hdt.exe"];

pub fn other_tracker_running() -> bool {
    running_exes().iter().any(|n| OTHER_TRACKERS.contains(&n.as_str()))
}

/// Our own exe's file name in lower case (the overlay stays shown while it is in front).
pub fn own_exe() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}
