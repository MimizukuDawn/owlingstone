"""Mises à jour en un clic.

Qui fait quoi :
- Le projet (le dossier qui contient bgtracker/ et update.bat) porte un numéro de version dans
  bgtracker/__init__.py. Owlingstone.exe connaît le sien (celui avec lequel il a été construit).
- newer_version() relit le numéro du projet : s'il est plus grand, une mise à jour est prête
  (par exemple après que Claude a modifié le code).
- start_update() lance update.bat en arrière-plan, sans fenêtre cmd (sortie dans USER_DIR/update.log) :
  il teste et reconstruit pendant que l'appli reste ouverte, puis la ferme, remplace l'exe et le relance.
  Mes données (historique, réglages...) sont ailleurs (USER_DIR) : rien n'est perdu.
- whats_new() : les lignes de CHANGELOG.md pour une version, montrées après la mise à jour.

Version téléchargée depuis GitHub (pas de projet à côté de l'exe) : voir RELEASES plus bas. Un fil en
arrière-plan regarde la dernière version publiée (au plus toutes les 6 h), la télécharge, vérifie son
empreinte SHA-256 et la pose à côté (Owlingstone.exe.new). Le bouton n'apparaît qu'une fois tout vérifié ;
start_update() échange alors les deux exe et lance le nouveau, sans fenêtre ni script.
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


UPDATE_LOG = USER_DIR / "update.log"


def run_hidden(args, cwd=None, log=UPDATE_LOG):
    """Lance un script sans fenêtre cmd ; sa sortie va dans update.log. Renvoie le processus."""
    log.parent.mkdir(parents=True, exist_ok=True)
    flags = getattr(subprocess, "CREATE_NO_WINDOW", 0) | getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
    with open(log, "w", encoding="utf-8") as out:
        return subprocess.Popen(args, cwd=cwd, stdin=subprocess.DEVNULL, stdout=out,  # nosec B603 B607
                                stderr=subprocess.STDOUT, creationflags=flags)


def start_update(source_dir=SOURCE_DIR):
    """Lance update.bat (ou le remplacement de la version GitHub) sans fenêtre.
    Renvoie le processus lancé, ou None si impossible."""
    if source_dir is None and getattr(sys, "frozen", False):
        return RELEASES.install()
    script = update_script(source_dir)
    if script is None:
        return None
    return run_hidden(["cmd", "/c", str(script), "quiet"], cwd=str(source_dir))


FAILED_MARK = USER_DIR / "update_failed.txt"


def take_failed_mark(mark=FAILED_MARK):
    """True (une seule fois) si update.bat a dû nous fermer puis a échoué : l'appli le dit au démarrage."""
    try:
        mark.unlink()
        return True
    except OSError:
        return False


def log_tail(lines=8, log=UPDATE_LOG):
    """Les dernières lignes utiles de update.log (pour dire pourquoi une mise à jour a échoué)."""
    try:
        text = log.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""
    keep = [line.rstrip() for line in text.splitlines() if line.strip()]
    return "\n".join(keep[-lines:])


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
ASSET_EXE = "Owlingstone.exe"       # le seul fichier d'une Release ; sa version = le tag (v1.2.3)
TAG = re.compile(r"^v(\d+(?:\.\d+){1,3})$")
AFTER_UPDATE = "OWLINGSTONE_AFTER_UPDATE"   # dit au nouvel exe d'attendre que l'ancien soit fermé
CHECK_EVERY = 6 * 3600              # secondes entre deux questions à GitHub
MAX_DOWNLOAD = 300 * 1024 * 1024    # refuse un téléchargement plus gros (anormal)


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
    """Réponse de l'API "latest release" -> {version, url, sha256, size} ou None si rien d'utilisable.
    L'empreinte SHA-256 est le champ "digest" que GitHub calcule lui-même pour chaque fichier publié :
    sans elle, pas de mise à jour."""
    if not isinstance(release, dict) or release.get("draft") or release.get("prerelease"):
        return None
    tag = TAG.match(str(release.get("tag_name") or ""))
    for asset in release.get("assets") or []:
        if not tag or not isinstance(asset, dict) or asset.get("name") != ASSET_EXE:
            continue
        url = asset.get("browser_download_url") or ""
        digest = str(asset.get("digest") or "")
        sha = digest[7:].lower() if digest.startswith("sha256:") else ""
        if not url.startswith(DOWNLOAD_PREFIX) or not re.fullmatch(r"[0-9a-f]{64}", sha):
            return None
        return {"version": tag.group(1), "url": url, "sha256": sha, "size": asset.get("size") or 0}
    return None


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


class ReleaseUpdater:
    """Cherche, télécharge et vérifie la dernière version GitHub dans un fil à part (jamais l'interface).
    `ready` = la version prête à installer (dict) une fois l'exe vérifié et posé à côté, sinon None.

    Un exe en cours peut être renommé sous Windows : install() met Owlingstone.exe de côté
    (Owlingstone.exe.old), met le nouveau à sa place et le lance ; le .old est supprimé au démarrage suivant.
    Un ancien dossier Owlingstone\\ (version zip) se met à jour de la même façon : seul l'exe change."""

    def __init__(self, current=__version__, exe_path=None, state_path=None):
        self.current = current
        self.exe_path = Path(exe_path or Path(sys.executable).resolve())
        self.state_path = Path(state_path or USER_DIR / "update_check.json")
        self.ready = None
        self.error = None
        self._thread = None

    @property
    def new_exe(self):
        return self.exe_path.with_name(self.exe_path.name + ".new")

    @property
    def old_exe(self):
        return self.exe_path.with_name(self.exe_path.name + ".old")

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
        if info["size"] > MAX_DOWNLOAD:
            raise ValueError("release too large")
        fd, tmp = tempfile.mkstemp(suffix=".exe", dir=self.state_path.parent)
        try:
            with os.fdopen(fd, "wb") as f:
                f.write(get(info["url"], MAX_DOWNLOAD, timeout=300))
            if sha256_of(tmp) != info["sha256"]:
                raise ValueError("checksum mismatch: download refused")
            shutil.move(tmp, self.new_exe)           # vérifié : posé à côté de l'exe en cours
        finally:
            try:
                os.remove(tmp)
            except OSError:
                pass
        self.ready = info
        return info

    def install(self):
        """Échange les exe et lance le nouveau (l'appli doit ensuite se fermer). None si impossible."""
        if not self.ready or not self.new_exe.exists():
            return None
        try:
            if self.old_exe.exists():
                self.old_exe.unlink()
            self.exe_path.rename(self.old_exe)
        except OSError:
            return None
        try:
            self.new_exe.rename(self.exe_path)
        except OSError:
            self.old_exe.rename(self.exe_path)       # retour en arrière : rien n'a changé
            return None
        env = dict(os.environ, **{AFTER_UPDATE: "1"})
        flags = getattr(subprocess, "DETACHED_PROCESS", 0) | getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
        return subprocess.Popen([str(self.exe_path)], env=env, close_fds=True,  # nosec B603 : notre exe vérifié
                                creationflags=flags)


def remove_old_exe(exe_path=None):
    """Après une mise à jour : supprime l'ancien exe (Owlingstone.exe.old) s'il traîne."""
    exe_path = Path(exe_path or Path(sys.executable).resolve())
    try:
        exe_path.with_name(exe_path.name + ".old").unlink()
    except OSError:
        pass


RELEASES = ReleaseUpdater()
