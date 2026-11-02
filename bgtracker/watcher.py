"""Suivi en direct de Power.log (comme `tail -f` sous Linux).

Qui fait quoi :
- Hearthstone crée un dossier Logs\\Hearthstone_<date> à chaque lancement du jeu et y écrit Power.log.
- find_power_log() trouve le Power.log du dossier le plus récent.
- LogWatcher.poll() lit seulement les lignes ajoutées depuis le dernier appel et les donne au parser.
  Si le jeu est relancé (nouveau dossier) ou si le fichier est recréé, on repart de zéro.
"""
import os
import time
from pathlib import Path

from .powerlog import PowerLogParser

DEFAULT_LOGS_DIR = Path(r"C:\Program Files (x86)\Hearthstone\Logs")


def find_power_log(logs_dir=DEFAULT_LOGS_DIR):
    """Power.log du dernier lancement du jeu, ou None si aucun."""
    folders = sorted(Path(logs_dir).glob("Hearthstone_*"), key=lambda p: p.stat().st_mtime)
    for folder in reversed(folders):            # du plus récent au plus ancien
        log = folder / "Power.log"
        if log.exists():
            return log
    return None


class LogWatcher:
    def __init__(self, logs_dir=DEFAULT_LOGS_DIR, on_new_game=None):
        self.logs_dir = logs_dir
        self.on_new_game = on_new_game
        self.parser = PowerLogParser(on_new_game=on_new_game)
        self.path = None        # fichier suivi
        self._pos = 0           # position (en octets) déjà lue dans le fichier
        self._partial = ""      # fin de ligne pas encore terminée par le jeu

    def poll(self):
        """Lit les nouvelles lignes. Renvoie True si quelque chose a été lu."""
        path = find_power_log(self.logs_dir)
        if path is None:
            return False
        if path != self.path or os.path.getsize(path) < self._pos:
            # Nouveau fichier (jeu relancé) ou fichier recréé plus petit : on relit depuis le début.
            self.path, self._pos, self._partial = path, 0, ""
            self.parser = PowerLogParser(on_new_game=self.on_new_game)
        with open(path, "rb") as f:             # binaire : la position en octets reste exacte
            f.seek(self._pos)
            data = f.read()
            self._pos = f.tell()
        if not data:
            return False
        text = self._partial + data.decode("utf-8", errors="replace")
        lines = text.split("\n")
        self._partial = lines.pop()             # le dernier morceau peut être une ligne inachevée
        for line in lines:
            self.parser.feed_line(line)
        return True

    @property
    def game(self):
        return self.parser.game

    def run(self, on_update, interval=0.5):
        """Boucle sans fin : appelle on_update(game) après chaque lecture qui a apporté du neuf."""
        while True:
            if self.poll() and self.game is not None:
                on_update(self.game)
            time.sleep(interval)
