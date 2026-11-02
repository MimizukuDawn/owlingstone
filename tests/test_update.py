"""Mises à jour : depuis GitHub Releases (exe seul, empreinte SHA-256) et depuis le projet (update.bat)."""
import hashlib
import json

import pytest

from bgtracker import ed25519, update
from bgtracker.update import ReleaseUpdater, pick_release, signed_sha256

EXE_URL = update.DOWNLOAD_PREFIX + "v9.9.9/Owlingstone.exe"
SIG_URL = update.DOWNLOAD_PREFIX + "v9.9.9/Owlingstone.exe.sig"
SEED = bytes(range(32))                       # clé de test (la vraie clé secrète n'est jamais ici)
TEST_KEY = ed25519.public_key(SEED).hex()


@pytest.fixture(autouse=True)
def test_key(monkeypatch):
    monkeypatch.setattr(update, "PUBLIC_KEYS", (TEST_KEY,))


def sig_file(data, version="9.9.9", seed=SEED):
    text = f"owlingstone-release 1\nversion {version}\nsha256 {hashlib.sha256(data).hexdigest()}\n"
    return (text + "signature " + ed25519.sign(seed, text.encode()).hex() + "\n").encode()


def release(data, digest=True, signed=True, **extra):
    asset = {"name": "Owlingstone.exe", "browser_download_url": EXE_URL, "size": len(data)}
    if digest:
        asset["digest"] = "sha256:" + hashlib.sha256(data).hexdigest()
    assets = [asset]
    if signed:
        assets.append({"name": "Owlingstone.exe.sig", "browser_download_url": SIG_URL, "size": 200})
    return {"tag_name": "v9.9.9", "assets": assets, **extra}


def files_for(new, exe=None, sig=None):
    return {update.API_LATEST: json.dumps(release(new)).encode(), EXE_URL: new if exe is None else exe,
            SIG_URL: sig_file(new) if sig is None else sig}


def updater(tmp_path, old=b"old exe"):
    exe = tmp_path / "Downloads" / "Owlingstone.exe"
    exe.parent.mkdir()
    exe.write_bytes(old)
    return ReleaseUpdater(current="1.0.0", exe_path=exe, state_path=tmp_path / "user" / "check.json")


def test_pick_release_reads_version_and_digest():
    info = pick_release(release(b"abc"))
    assert info["version"] == "9.9.9" and info["digest"] == hashlib.sha256(b"abc").hexdigest()
    assert info["sig_url"] == SIG_URL


def test_unsigned_release_is_ignored():
    assert pick_release(release(b"abc", signed=False)) is None


def test_signature_checks():
    assert signed_sha256(sig_file(b"x").decode(), "9.9.9") == hashlib.sha256(b"x").hexdigest()
    with pytest.raises(ValueError):                       # signée pour une autre version
        signed_sha256(sig_file(b"x", version="1.0.0").decode(), "9.9.9")
    with pytest.raises(ValueError):                       # signée par une autre clé
        signed_sha256(sig_file(b"x", seed=bytes(32)).decode(), "9.9.9")
    forged = sig_file(b"x").decode().replace(hashlib.sha256(b"x").hexdigest(), hashlib.sha256(b"evil").hexdigest())
    with pytest.raises(ValueError):                       # empreinte changée après signature
        signed_sha256(forged, "9.9.9")


def test_real_public_key_is_valid():
    import re
    from pathlib import Path
    keys = re.findall(r'"([0-9a-f]{64})"', Path(update.__file__).read_text(encoding="utf-8"))
    assert keys and all(ed25519._decompress(bytes.fromhex(k)) for k in keys)


def test_pick_release_refuses_other_hosts_drafts_bad_tags_and_missing_checksum():
    r = release(b"abc")
    r["assets"][0]["browser_download_url"] = "https://evil.example/Owlingstone.exe"
    assert pick_release(r) is None
    assert pick_release(release(b"abc", draft=True)) is None
    assert pick_release(release(b"abc", prerelease=True)) is None
    assert pick_release(release(b"abc", digest=False))["digest"] == ""   # la signature suffit
    assert pick_release(release(b"abc", tag_name="stats")) is None


def test_get_refuses_foreign_urls():
    with pytest.raises(ValueError):
        update._get("https://evil.example/x", 10)


def test_download_verify_and_swap(tmp_path, monkeypatch):
    up = updater(tmp_path)
    new = b"new exe"
    files = files_for(new)
    assert up.check_once(get=lambda url, limit, timeout=20: files[url], now=10**9)["version"] == "9.9.9"
    assert up.new_exe.read_bytes() == new and up.exe_path.read_bytes() == b"old exe"
    started = []
    monkeypatch.setattr(update.subprocess, "Popen", lambda args, **kw: started.append((args, kw)) or "proc")
    assert up.install() == "proc"
    assert up.exe_path.read_bytes() == new and up.old_exe.read_bytes() == b"old exe" and not up.new_exe.exists()
    assert started[0][0] == [str(up.exe_path)] and started[0][1]["env"][update.AFTER_UPDATE] == "1"
    update.remove_old_exe(up.exe_path)
    assert not up.old_exe.exists()


def test_tampered_download_is_refused(tmp_path):
    up = updater(tmp_path)
    files = files_for(b"new", exe=b"evil")
    with pytest.raises(ValueError):
        up.check_once(get=lambda url, limit, timeout=20: files[url], now=10**9)
    assert up.ready is None and not up.new_exe.exists()
    assert up.install() is None and up.exe_path.read_bytes() == b"old exe"


def test_release_signed_by_another_key_is_refused(tmp_path):
    """Même avec le compte GitHub volé (exe, signature et digest cohérents), sans notre clé : refusé."""
    up = updater(tmp_path)
    files = files_for(b"evil", sig=sig_file(b"evil", seed=bytes(32)))
    with pytest.raises(ValueError):
        up.check_once(get=lambda url, limit, timeout=20: files[url], now=10**9)
    assert up.ready is None and not up.new_exe.exists()


def test_same_version_is_not_downloaded(tmp_path):
    up = updater(tmp_path)
    up.current = "9.9.9"
    calls = []
    files = {update.API_LATEST: json.dumps(release(b"x")).encode()}
    assert up.check_once(get=lambda url, limit, timeout=20: calls.append(url) or files[url], now=10**9) is None
    assert calls == [update.API_LATEST]


def test_check_once_waits_between_checks(tmp_path):
    calls = []
    up = updater(tmp_path)
    get = lambda url, limit, timeout=20: calls.append(url) or b"{}"
    up.check_once(get=get, now=10**9)
    up.check_once(get=get, now=10**9 + 60)
    assert len(calls) == 1


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


def test_cards_fallback_offline_and_background_refresh(tmp_path, monkeypatch):
    """Premier lancement sans internet : la copie incluse sert, rien ne plante ; la mise à jour se fait
    dans un fil à part et remplace la base une fois chargée."""
    import gzip
    from bgtracker import cards
    monkeypatch.setattr(cards, "CARDS_PATH", tmp_path / "cards.json")
    monkeypatch.setattr(cards, "_db", None)
    monkeypatch.setattr(cards, "_state", {"thread": None, "last_try": 0.0, "source": None})
    monkeypatch.delenv(cards.NO_DOWNLOAD, raising=False)

    def offline(path=None):
        raise OSError("no network")
    monkeypatch.setattr(cards, "download", offline)
    db = cards.get_db()
    assert cards._state["source"] == cards.FALLBACK_PATH and len(db.by_id) > 1000
    cards._state["thread"].join(5)
    assert cards.get_db() is db                          # échec réseau : on garde la copie

    new = [{"id": "NEW_001", "dbfId": 1, "name": "Brand new"}] + [{"id": f"X_{i}", "dbfId": i + 2} for i in range(1000)]

    def online(path=None):
        cards.CARDS_PATH.write_text(json.dumps(new), encoding="utf-8")
    monkeypatch.setattr(cards, "download", online)
    assert db.get("NEW_001") is None                     # inconnue (la mise à jour suit)
    t = cards.refresh_in_background(force=True)
    t.join(5)
    assert cards.get_db().get("NEW_001").name == "Brand new"
    with gzip.open(cards.FALLBACK_PATH, "rt", encoding="utf-8") as f:
        assert isinstance(json.load(f), list)
