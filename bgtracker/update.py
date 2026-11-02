"""Mises à jour en un clic.

Qui fait quoi :
- Le projet (le dossier qui contient bgtracker/ et update.bat) porte un numéro de version dans
  bgtracker/__init__.py. Owlingstone.exe connaît le sien (celui avec lequel il a été construit).
- newer_version() relit le numéro du projet : s'il est plus grand, une mise à jour est prête
  (par exemple après que Claude a modifié le code).
- start_update() lance update.bat dans sa propre fenêtre : il ferme Owlingstone, teste, reconstruit
  l'exe et le relance. Mes données (historique, réglages...) sont ailleurs (USER_DIR) : rien n'est perdu.
- whats_new() : les lignes de CHANGELOG.md pour une version, montrées après la mise à jour.

Version téléchargée depuis GitHub (pas de projet à côté de l'exe) : voir RELEASES plus bas. Un fil en
arrière-plan regarde la dernière version publiée (au plus toutes les 6 h), la télécharge, vérifie son
empreinte SHA-256 et la décompresse à côté de l'appli. Le bouton n'apparaît qu'une fois tout vérifié ;
start_update() remplace alors le dossier de l'appli et la relance (l'ancien dossier est gardé en cas d'échec).
"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import zipfile
from pathlib import Path
from urllib.request import Request, urlopen

from . import __version__
from .paths import DATA_DIR, SOURCE_DIR, USER_DIR

_VERSION = re.compile(r'__version__\s*=\s*"([^"]+)"')


def parse(version):
    """"1.10.2" -> (1, 10, 2), pour comparer correctement (1.10 > 1.9)."""
    return tuple(int(x) for x in re.findall(r"\d+", version or "0"))


def source_version(source_dir=SOURCE_DIR):
    try:
        m = _VERSION.search((source_dir / "bgtracker" / "__init__.py").read_text(encoding="utf-8"))
    except (OSError, TypeError):
        return None
    return m.group(1) if m else None


def newer_version(current=__version__, source_dir=SOURCE_DIR):
    """Version plus récente que celle qui tourne et prête à installer (exe seulement), sinon None.
    Exe construit dans le projet : la version du projet. Exe téléchargé : la dernière version GitHub."""
    if not getattr(sys, "frozen", False):
        return None              # lancé avec Python : on exécute déjà le code du projet
    if source_dir is None:
        RELEASES.start()
        ready = RELEASES.ready
        return ready["version"] if ready and parse(ready["version"]) > parse(current) else None
    found = source_version(source_dir)
    return found if found and parse(found) > parse(current) else None


def update_script(source_dir=SOURCE_DIR):
    path = source_dir / "update.bat" if source_dir else None
    return path if path and path.exists() else None


def start_update(source_dir=SOURCE_DIR):
    """Lance update.bat (ou le remplacement de la version GitHub). Renvoie False si impossible."""
    if source_dir is None and getattr(sys, "frozen", False):
        return RELEASES.install()
    script = update_script(source_dir)
    if script is None:
        return False
    subprocess.Popen(["cmd", "/c", str(script)], cwd=str(source_dir),
                     creationflags=getattr(subprocess, "CREATE_NEW_CONSOLE", 0))
    return True


def whats_new(version=__version__, changelog=None):
    """Lignes "- ..." de la section "## <version>" de CHANGELOG.md (liste vide si absente)."""
    path = changelog or (SOURCE_DIR or DATA_DIR.parent) / "CHANGELOG.md"
    try:
        text = path.read_text(encoding="utf-8")
    except OSError:
        return []
    lines, inside = [], False
    for line in text.splitlines():
        if line.startswith("## "):
            inside = line[3:].split()[0] == version
        elif inside and line.strip().startswith("- "):
            lines.append(line.strip()[2:])
    return lines


# ------------------------------------------------------------ mises à jour depuis GitHub Releases
REPO = "MimizukuDawn/owlingstone"
API_LATEST = f"https://api.github.com/repos/{REPO}/releases/latest"
DOWNLOAD_PREFIX = f"https://github.com/{REPO}/releases/download/"
ASSET = re.compile(r"^Owlingstone-(\d+(?:\.\d+){1,3})-windows\.zip$")
CHECK_EVERY = 6 * 3600              # secondes entre deux questions à GitHub
MAX_ZIP = 400 * 1024 * 1024         # refuse un téléchargement plus gros (anormal)


def _get(url, limit, timeout=20):
    """Contenu d'une URL https de GitHub, au plus `limit` octets (sinon ValueError)."""
    if not url.startswith(("https://api.github.com/", DOWNLOAD_PREFIX)):
        raise ValueError(f"unexpected URL {url}")
    req = Request(url, headers={"Accept": "application/vnd.github+json", "User-Agent": "Owlingstone-updater"})
    with urlopen(req, timeout=timeout) as resp:  # nosec B310 : https, hôte vérifié ci-dessus
        data = resp.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"{url} is larger than {limit} bytes")
    return data


def pick_release(release):
    """Réponse de l'API "latest release" -> {version, url, sha256} ou None si rien d'utilisable.
    L'empreinte vient du champ "digest" de GitHub, sinon du fichier <zip>.sha256 publié avec."""
    if not isinstance(release, dict) or release.get("draft") or release.get("prerelease"):
        return None
    assets = {a.get("name"): a for a in release.get("assets") or [] if isinstance(a, dict)}
    for name, asset in assets.items():
        m = ASSET.match(name or "")
        url = asset.get("browser_download_url") or ""
        if not m or not url.startswith(DOWNLOAD_PREFIX):
            continue
        digest = asset.get("digest") or ""
        sha = digest[7:] if digest.startswith("sha256:") else None
        side = assets.get(name + ".sha256")
        side_url = (side or {}).get("browser_download_url") or ""
        if sha is None and not side_url.startswith(DOWNLOAD_PREFIX):
            return None
        return {"version": m.group(1), "url": url, "sha256": sha, "sha_url": side_url or None,
                "size": asset.get("size") or 0}
    return None


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def safe_extract(zip_path, dest):
    """Décompresse sans laisser un fichier sortir de `dest` (noms en ../ ou absolus refusés)."""
    dest = Path(dest).resolve()
    with zipfile.ZipFile(zip_path) as z:
        for member in z.infolist():
            target = (dest / member.filename).resolve()
            if target != dest and dest not in target.parents:
                raise ValueError(f"unsafe path in zip: {member.filename}")
        z.extractall(dest)


def swap_script(pid, install_dir, new_dir, old_dir, exe_name="Owlingstone.exe"):
    """Petit .bat : attend la fin de l'appli, échange les dossiers, relance (revient en arrière si raté)."""
    return "\r\n".join([
        "@echo off",
        "chcp 65001 >nul",
        "title Owlingstone update",
        ":wait",
        f'tasklist /fi "PID eq {int(pid)}" | find "{int(pid)}" >nul && (ping -n 2 127.0.0.1 >nul & goto wait)',
        f'if exist "{old_dir}" rmdir /s /q "{old_dir}"',
        f'move "{install_dir}" "{old_dir}" >nul || goto failed',
        f'move "{new_dir}" "{install_dir}" >nul || (move "{old_dir}" "{install_dir}" >nul & goto failed)',
        f'start "" "{install_dir}\\{exe_name}"',
        f'rmdir /s /q "{old_dir}"',
        "exit /b 0",
        ":failed",
        f'start "" "{install_dir}\\{exe_name}"',
        "exit /b 1",
        ""])


class ReleaseUpdater:
    """Cherche, télécharge et vérifie la dernière version GitHub dans un fil à part (jamais l'interface).
    `ready` = la version prête à installer (dict) une fois le zip vérifié et décompressé, sinon None."""

    def __init__(self, current=__version__, install_dir=None, state_path=None):
        self.current = current
        self.install_dir = Path(install_dir or Path(sys.executable).resolve().parent)
        self.state_path = Path(state_path or USER_DIR / "update_check.json")
        self.ready = None
        self.error = None
        self._thread = None

    @property
    def new_dir(self):
        return self.install_dir.with_name(self.install_dir.name + ".new")

    def start(self):
        if self._thread is None:
            self._thread = threading.Thread(target=self._loop, name="release-updater", daemon=True)
            self._thread.start()

    def _loop(self):
        while True:
            try:
                self.check_once()
            except Exception as e:    # réseau coupé, GitHub indisponible... on réessaiera plus tard
                self.error = str(e)
            time.sleep(CHECK_EVERY)

    def _last_check(self):
        try:
            return float(json.loads(self.state_path.read_text(encoding="utf-8")).get("checked", 0))
        except (OSError, ValueError, AttributeError):
            return 0.0

    def check_once(self, get=_get, now=None):
        now = time.time() if now is None else now
        if self.ready or now - self._last_check() < CHECK_EVERY:
            return self.ready
        self.state_path.parent.mkdir(parents=True, exist_ok=True)
        self.state_path.write_text(json.dumps({"checked": now}), encoding="utf-8")
        info = pick_release(json.loads(get(API_LATEST, 1_000_000)))
        if not info or parse(info["version"]) <= parse(self.current):
            return None
        if info["size"] > MAX_ZIP:
            raise ValueError("release too large")
        expected = info["sha256"] or get(info["sha_url"], 1000).decode("ascii", "replace").split()[0]
        if not re.fullmatch(r"[0-9a-f]{64}", expected.lower()):
            raise ValueError("bad checksum format")
        fd, tmp = tempfile.mkstemp(suffix=".zip", dir=self.state_path.parent)
        try:
            with os.fdopen(fd, "wb") as f:
                f.write(get(info["url"], MAX_ZIP, timeout=300))
            if sha256_of(tmp) != expected.lower():
                raise ValueError("checksum mismatch: download refused")
            stage = self.install_dir.with_name(self.install_dir.name + ".unpack")
            if stage.exists():
                shutil.rmtree(stage)
            safe_extract(tmp, stage)
            inner = stage / "Owlingstone"
            if not (inner / "Owlingstone.exe").exists():
                raise ValueError("Owlingstone.exe missing from the release")
            if self.new_dir.exists():
                shutil.rmtree(self.new_dir)
            inner.rename(self.new_dir)
            stage.rmdir()
        finally:
            try:
                os.remove(tmp)
            except OSError:
                pass
        self.ready = info
        return info

    def install(self):
        """Écrit et lance le script d'échange, puis l'appli doit se fermer. False si rien n'est prêt."""
        if not self.ready or not self.new_dir.exists():
            return False
        old = self.install_dir.with_name(self.install_dir.name + ".old")
        script = self.state_path.parent / "swap_update.bat"
        script.write_text(swap_script(os.getpid(), self.install_dir, self.new_dir, old), encoding="utf-8")
        subprocess.Popen(["cmd", "/c", str(script)],  # nosec B603 B607 : notre propre script, chemins locaux
                         creationflags=getattr(subprocess, "CREATE_NEW_CONSOLE", 0))
        return True


RELEASES = ReleaseUpdater()
