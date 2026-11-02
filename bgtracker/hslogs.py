"""Where Hearthstone writes its logs, and making sure it writes Power.log at all.

Who does what:
- logs_dir_candidates() lists, best first: the folder chosen by the player (tray menu "Choose Hearthstone
  folder…", saved as "hearthstone_dir" in settings.json), the folder of the running Hearthstone.exe, the
  install folder written in the Windows registry by Battle.net, then the usual install folders.
- find_logs_dir() returns the first one that exists (or None): Hearthstone installed on D:\\Games works too.
- ensure_log_config() adds the [Power] section to %LOCALAPPDATA%\\Blizzard\\Hearthstone\\log.config if it is
  missing, like other trackers do: without it the game writes no Power.log. Hearthstone reads that file
  when it starts, so a game already running must be restarted once.
"""
import ctypes
import json
import os
from pathlib import Path

from .paths import USER_DIR

SETTINGS_PATH = USER_DIR / "settings.json"
DEFAULT_INSTALLS = [Path(r"C:\Program Files (x86)\Hearthstone"), Path(r"C:\Program Files\Hearthstone")]
UNINSTALL_KEYS = [r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Hearthstone",
                  r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Hearthstone"]
POWER_SECTION = "[Power]\nLogLevel=1\nFilePrinting=true\nConsolePrinting=false\nScreenPrinting=false\nVerbose=true\n"


def chosen_dir(settings_path=SETTINGS_PATH):
    try:
        value = json.loads(settings_path.read_text(encoding="utf-8")).get("hearthstone_dir")
    except (OSError, ValueError, AttributeError):
        return None
    return Path(value) if isinstance(value, str) and value else None


def as_logs_dir(folder):
    """The player may pick the install folder or its Logs folder: both are fine."""
    folder = Path(folder)
    return folder if folder.name.lower() == "logs" else folder / "Logs"


def running_install():
    """Folder of the running Hearthstone.exe (Windows), or None."""
    try:
        user32, kernel32 = ctypes.windll.user32, ctypes.windll.kernel32
    except AttributeError:
        return None
    try:
        hwnd = user32.FindWindowW("UnityWndClass", "Hearthstone")
        if not hwnd:
            return None
        pid = ctypes.c_ulong()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        handle = kernel32.OpenProcess(0x1000, False, pid.value)    # 0x1000 = limited query rights
        if not handle:
            return None
        buf, size = ctypes.create_unicode_buffer(1024), ctypes.c_ulong(1024)
        ok = kernel32.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size))
        kernel32.CloseHandle(handle)
        return Path(buf.value).parent if ok and buf.value else None
    except OSError:
        return None


def registry_install():
    """InstallLocation that Battle.net writes in the registry, or None."""
    try:
        import winreg
    except ImportError:
        return None
    for key in UNINSTALL_KEYS:
        try:
            with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key) as k:
                value, _ = winreg.QueryValueEx(k, "InstallLocation")
            if value:
                return Path(value)
        except OSError:
            continue
    return None


def logs_dir_candidates(settings_path=SETTINGS_PATH, running=running_install, registry=registry_install):
    found = []
    chosen = chosen_dir(settings_path)
    if chosen:
        found.append(as_logs_dir(chosen))
    for install in (running(), registry(), *DEFAULT_INSTALLS):
        if install:
            found.append(as_logs_dir(install))
    unique = []
    for path in found:
        if path not in unique:
            unique.append(path)
    return unique


def find_logs_dir(**kw):
    """First candidate whose Logs folder (or Hearthstone install) exists, else None."""
    for logs in logs_dir_candidates(**kw):
        if logs.is_dir() or (logs.parent / "Hearthstone.exe").exists():
            return logs
    return None


def log_config_path():
    base = os.environ.get("LOCALAPPDATA")
    return Path(base) / "Blizzard" / "Hearthstone" / "log.config" if base else None


def ensure_log_config(path=None):
    """'ok' if Power logging is already on, 'written' if we just turned it on (restart the game once),
    None if we could not (not Windows, or the file cannot be written)."""
    path = path or log_config_path()
    if path is None:
        return None
    try:
        text = path.read_text(encoding="utf-8", errors="replace") if path.exists() else ""
        if "[power]" in text.lower():
            return "ok"
        path.parent.mkdir(parents=True, exist_ok=True)
        with open(path, "a", encoding="utf-8", newline="\r\n") as f:
            f.write(("\n" if text and not text.endswith("\n") else "") + POWER_SECTION)
        return "written"
    except OSError:
        return None
