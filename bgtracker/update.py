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
arrière-plan regarde la dernière version publiée (au plus toutes les 6 h), la télécharge et vérifie son
empreinte SHA-256. Le bouton n'apparaît qu'une fois tout vérifié. Deux formes :
- Owlingstone.exe seul (un seul fichier, la forme normale) : le nouvel exe est posé à côté
  (Owlingstone.exe.new) ; start_update() renomme l'exe en cours en .old (Windows le permet), met le
  nouveau à sa place et le lance ; le .old est supprimé au démarrage suivant.
- dossier Owlingstone\\ (le zip) : le dossier est remplacé par un petit script sans fenêtre.
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
ASSET = re.compile(r"^Owlingstone-(\d+(?:\.\d+){1,3})-windows\.zip$")
ASSET_EXE = "Owlingstone.exe"       # l'exe en un seul fichier ; sa version = le tag de la Release
TAG = re.compile(r"^v(\d+(?:\.\d+){1,3})$")
AFTER_UPDATE = "OWLINGSTONE_AFTER_UPDATE"   # dit au nouvel exe d'attendre que l'ancien soit fermé
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


def pick_release(release, onefile=False):
    """Réponse de l'API "latest release" -> {version, url, sha256} ou None si rien d'utilisable.
    onefile : l'exe seul (Owlingstone.exe), sinon le zip. L'empreinte vient du champ "digest" de
    GitHub, sinon du fichier <nom>.sha256 publié avec."""
    if not isinstance(release, dict) or release.get("draft") or release.get("prerelease"):
        return None
    tag = TAG.match(str(release.get("tag_name") or ""))
    assets = {a.get("name"): a for a in release.get("assets") or [] if isinstance(a, dict)}
    for name, asset in assets.items():
        if onefile:
            m = tag if name == ASSET_EXE else None
        else:
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

    def __init__(self, current=__version__, install_dir=None, state_path=None, exe_path=None, onefile=None):
        self.current = current
        self.exe_path = Path(exe_path or Path(sys.executable).resolve())
        self.install_dir = Path(install_dir or self.exe_path.parent)
        self.onefile = is_onefile() if onefile is None else onefile
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
        info = pick_release(json.loads(get(API_LATEST, 1_000_000)), self.onefile)
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
            if self.onefile:
                shutil.move(tmp, self.new_exe)       # vérifié : posé à côté de l'exe en cours
                self.ready = info
                return info
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
        """Écrit et lance (sans fenêtre) le script d'échange, puis l'appli doit se fermer.
        Renvoie le processus, ou None si rien n'est prêt."""
        if self.onefile:
            return self._install_onefile()
        if not self.ready or not self.new_dir.exists():
            return None
        old = self.install_dir.with_name(self.install_dir.name + ".old")
        script = self.state_path.parent / "swap_update.bat"
        script.write_text(swap_script(os.getpid(), self.install_dir, self.new_dir, old), encoding="utf-8")
        return run_hidden(["cmd", "/c", str(script)])  # notre propre script, chemins locaux


    def _install_onefile(self):
        """Un exe en cours peut être renommé sous Windows : on le met de côté, le nouveau prend sa place."""
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


def is_onefile():
    """Exe en un seul fichier : PyInstaller le décompresse dans un dossier temporaire (pas _internal\\)."""
    if not getattr(sys, "frozen", False):
        return False
    exe_dir = Path(sys.executable).resolve().parent
    return not Path(getattr(sys, "_MEIPASS", exe_dir)).resolve().is_relative_to(exe_dir)


def remove_old_exe(exe_path=None):
    """Après une mise à jour de l'exe seul : supprime l'ancien (Owlingstone.exe.old) s'il traîne."""
    exe_path = Path(exe_path or Path(sys.executable).resolve())
    try:
        exe_path.with_name(exe_path.name + ".old").unlink()
    except OSError:
        pass


RELEASES = ReleaseUpdater()
