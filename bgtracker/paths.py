"""Où sont les fichiers, en Python comme en .exe. Deux sortes de fichiers :

- DATA_DIR = les données de l'application, qui changent avec les versions : comps.yaml, heroes.yaml,
  rules.yaml (et subsets.json de départ). C'est le dossier data/ du projet, aussi pour l'exe construit
  dans le projet (dist\\Owlingstone\\ -> on remonte jusqu'au data/ du projet) : modifier rules.yaml
  s'applique tout de suite, sans reconstruire. Exe déplacé ailleurs : la copie incluse dans l'exe.
- USER_DIR = MES données, qu'une mise à jour ne touche jamais : historique, réglages, images des cartes,
  cards.json, races apprises (subsets.json), error.log. Sous Windows : %APPDATA%\\Owlingstone.
  Au premier lancement, on y copie ce qui se trouvait avant dans data/ (rien n'est supprimé).
"""
import os
import shutil
import sys
from pathlib import Path

PROJECT_DIR = Path(__file__).resolve().parent.parent
# Fichiers qui vivaient dans data/ avant la version 1.1 et qui sont maintenant dans USER_DIR.
USER_FILES = ["history.jsonl", "settings.json", "subsets.json", "cards.json", "error.log", "tiles"]


def _source_dir():
    """Dossier du projet (celui qui contient bgtracker/ et data/), ou None si introuvable."""
    if not getattr(sys, "frozen", False):           # "frozen" = lancé depuis un .exe PyInstaller
        return PROJECT_DIR
    exe_dir = Path(sys.executable).resolve().parent
    for folder in [exe_dir, *exe_dir.parents]:
        if (folder / "data" / "comps.yaml").exists() and (folder / "bgtracker").is_dir():
            return folder
    return None


def _data_dir():
    if SOURCE_DIR is not None:
        return SOURCE_DIR / "data"
    return Path(getattr(sys, "_MEIPASS", PROJECT_DIR)) / "data"   # copie incluse dans l'exe


def _user_dir():
    if os.environ.get("BGTRACKER_HOME"):            # pour les tests, ou pour tout mettre ailleurs
        return Path(os.environ["BGTRACKER_HOME"])
    if os.environ.get("APPDATA"):                   # Windows
        return Path(os.environ["APPDATA"]) / "Owlingstone"
    return Path.home() / ".owlingstone"


def adopt_old_user_dir(user_dir):
    """L'appli s'appelait BG Tracker : au premier lancement d'Owlingstone, on recopie %APPDATA%\\BGTracker
    (historique, réglages...) dans le nouveau dossier. L'ancien dossier n'est ni modifié ni supprimé."""
    old = user_dir.with_name("BGTracker") if user_dir.name == "Owlingstone" else None
    if old is None or user_dir.exists() or not old.is_dir():
        return False
    shutil.copytree(old, user_dir)
    return True


def migrate(old_dir, user_dir):
    """Copie les anciens fichiers de data/ vers USER_DIR, une seule fois et sans rien écraser."""
    marker = user_dir / ".migrated"
    if marker.exists():
        return []
    user_dir.mkdir(parents=True, exist_ok=True)
    copied = []
    for name in USER_FILES:
        src, dst = old_dir / name, user_dir / name
        if not src.exists() or dst.exists():
            continue
        try:
            if src.is_dir():
                shutil.copytree(src, dst)
            else:
                shutil.copy2(src, dst)
            copied.append(name)
        except OSError:
            pass
    marker.write_text("copied from " + str(old_dir) + ": " + ", ".join(copied), encoding="utf-8")
    return copied


SOURCE_DIR = _source_dir()
DATA_DIR = _data_dir()
USER_DIR = _user_dir()
try:
    adopt_old_user_dir(USER_DIR)
    migrate(DATA_DIR, USER_DIR)
except OSError:
    pass
