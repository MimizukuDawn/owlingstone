"""Mises à jour depuis GitHub Releases : choix du fichier, empreinte, décompression sûre."""
import hashlib
import io
import json
import zipfile

import pytest

from bgtracker import update
from bgtracker.update import ReleaseUpdater, pick_release, safe_extract, swap_script

URL = update.DOWNLOAD_PREFIX + "v9.9.9/Owlingstone-9.9.9-windows.zip"


def make_zip(names):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        for n in names:
            z.writestr(n, "x")
    return buf.getvalue()


def release(data, digest=True, **extra):
    asset = {"name": "Owlingstone-9.9.9-windows.zip", "browser_download_url": URL, "size": len(data)}
    if digest:
        asset["digest"] = "sha256:" + hashlib.sha256(data).hexdigest()
    return {"tag_name": "v9.9.9", "assets": [asset], **extra}


def test_pick_release_reads_version_and_digest():
    info = pick_release(release(b"abc"))
    assert info["version"] == "9.9.9" and info["sha256"] == hashlib.sha256(b"abc").hexdigest()


def test_pick_release_refuses_other_hosts_drafts_and_missing_checksum():
    r = release(b"abc")
    r["assets"][0]["browser_download_url"] = "https://evil.example/Owlingstone-9.9.9-windows.zip"
    assert pick_release(r) is None
    assert pick_release(release(b"abc", draft=True)) is None
    assert pick_release(release(b"abc", digest=False)) is None


def test_get_refuses_foreign_urls():
    with pytest.raises(ValueError):
        update._get("https://evil.example/x", 10)


def test_safe_extract_blocks_zip_slip(tmp_path):
    bad = tmp_path / "bad.zip"
    bad.write_bytes(make_zip(["../evil.txt"]))
    with pytest.raises(ValueError):
        safe_extract(bad, tmp_path / "out")
    assert not (tmp_path / "evil.txt").exists()


def updater(tmp_path):
    install = tmp_path / "Owlingstone"
    install.mkdir()
    return ReleaseUpdater(current="1.0.0", install_dir=install, state_path=tmp_path / "user" / "check.json")


def test_check_once_downloads_verifies_and_unpacks(tmp_path):
    data = make_zip(["Owlingstone/Owlingstone.exe", "Owlingstone/_internal/x.dll"])
    files = {update.API_LATEST: json.dumps(release(data)).encode(), URL: data}
    up = updater(tmp_path)
    info = up.check_once(get=lambda url, limit, timeout=20: files[url], now=10**9)
    assert info["version"] == "9.9.9" and up.ready
    assert (tmp_path / "Owlingstone.new" / "Owlingstone.exe").exists()


def test_check_once_refuses_tampered_download(tmp_path):
    data = make_zip(["Owlingstone/Owlingstone.exe"])
    files = {update.API_LATEST: json.dumps(release(data)).encode(), URL: data + b"tampered"}
    up = updater(tmp_path)
    with pytest.raises(ValueError):
        up.check_once(get=lambda url, limit, timeout=20: files[url], now=10**9)
    assert up.ready is None and not (tmp_path / "Owlingstone.new").exists()


def test_check_once_waits_between_checks(tmp_path):
    calls = []
    up = updater(tmp_path)
    get = lambda url, limit, timeout=20: calls.append(url) or b"{}"
    up.check_once(get=get, now=10**9)
    up.check_once(get=get, now=10**9 + 60)
    assert len(calls) == 1


def test_swap_script_quotes_paths_and_rolls_back():
    text = swap_script(42, r"C:\Games\Owlingstone", r"C:\Games\Owlingstone.new", r"C:\Games\Owlingstone.old")
    assert 'move "C:\\Games\\Owlingstone" "C:\\Games\\Owlingstone.old"' in text
    assert ":failed" in text and "PID eq 42" in text


def test_old_bgtracker_data_is_copied_once(tmp_path):
    from bgtracker.paths import adopt_old_user_dir
    old = tmp_path / "BGTracker"
    old.mkdir()
    (old / "history.jsonl").write_text("{}\n")
    new = tmp_path / "Owlingstone"
    assert adopt_old_user_dir(new) and (new / "history.jsonl").exists() and old.exists()
    assert not adopt_old_user_dir(new)


def test_source_update_runs_without_a_window(tmp_path, monkeypatch):
    calls = []
    monkeypatch.setattr(update.subprocess, "Popen", lambda args, **kw: calls.append((args, kw)) or "proc")
    (tmp_path / "update.bat").write_text("@echo off", encoding="utf-8")
    monkeypatch.setattr(update, "UPDATE_LOG", tmp_path / "update.log")
    monkeypatch.setattr(update.sys, "frozen", False, raising=False)
    assert update.start_update(tmp_path) == "proc"
    args, kw = calls[0]
    assert args[-1] == "quiet" and kw["stdin"] == update.subprocess.DEVNULL
    assert kw["creationflags"] & getattr(update.subprocess, "CREATE_NO_WINDOW", 0) == \
        getattr(update.subprocess, "CREATE_NO_WINDOW", 0)
    assert update.start_update(tmp_path / "missing") is None


def test_failed_mark_and_log_tail(tmp_path):
    mark, log = tmp_path / "update_failed.txt", tmp_path / "update.log"
    assert not update.take_failed_mark(mark)
    mark.write_text("failed", encoding="utf-8")
    assert update.take_failed_mark(mark) and not mark.exists()
    log.write_text("a\n\nb\nUPDATE FAILED\n", encoding="utf-8")
    assert update.log_tail(2, log) == "b\nUPDATE FAILED"
    assert update.log_tail(log=tmp_path / "none.log") == ""


EXE_URL = update.DOWNLOAD_PREFIX + "v9.9.9/Owlingstone.exe"


def exe_release(data):
    r = release(make_zip(["Owlingstone/Owlingstone.exe"]))
    r["assets"].append({"name": "Owlingstone.exe", "browser_download_url": EXE_URL, "size": len(data),
                        "digest": "sha256:" + hashlib.sha256(data).hexdigest()})
    return r


def test_single_exe_download_and_swap(tmp_path, monkeypatch):
    exe = tmp_path / "Downloads" / "Owlingstone.exe"
    exe.parent.mkdir()
    exe.write_bytes(b"old exe")
    new = b"new exe"
    files = {update.API_LATEST: json.dumps(exe_release(new)).encode(), EXE_URL: new}
    up = ReleaseUpdater(current="1.0.0", exe_path=exe, state_path=tmp_path / "user" / "check.json", onefile=True)
    assert pick_release(exe_release(new), onefile=True)["url"] == EXE_URL
    assert up.check_once(get=lambda url, limit, timeout=20: files[url], now=10**9)["version"] == "9.9.9"
    assert up.new_exe.read_bytes() == new and exe.read_bytes() == b"old exe"
    started = []
    monkeypatch.setattr(update.subprocess, "Popen", lambda args, **kw: started.append((args, kw)) or "proc")
    assert up.install() == "proc"
    assert exe.read_bytes() == new and up.old_exe.read_bytes() == b"old exe" and not up.new_exe.exists()
    assert started[0][0] == [str(exe)] and started[0][1]["env"][update.AFTER_UPDATE] == "1"
    update.remove_old_exe(exe)
    assert not up.old_exe.exists()


def test_single_exe_refuses_tampered_download(tmp_path):
    exe = tmp_path / "Owlingstone.exe"
    exe.write_bytes(b"old")
    files = {update.API_LATEST: json.dumps(exe_release(b"new")).encode(), EXE_URL: b"evil"}
    up = ReleaseUpdater(current="1.0.0", exe_path=exe, state_path=tmp_path / "user" / "c.json", onefile=True)
    with pytest.raises(ValueError):
        up.check_once(get=lambda url, limit, timeout=20: files[url], now=10**9)
    assert not up.new_exe.exists() and up.install() is None and exe.read_bytes() == b"old"
