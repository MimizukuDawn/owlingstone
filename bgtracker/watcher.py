"""Suivi en direct de Power.log (comme `tail -f` sous Linux).

Qui fait quoi :
- Hearthstone crée un dossier Logs\\Hearthstone_<date> à chaque lancement du jeu et y écrit Power.log.
- find_power_log() trouve le Power.log du dossier le plus récent.
- Le dossier Logs est cherché par hslogs.find_logs_dir() (dossier choisi, jeu lancé, registre, dossiers
  habituels), de nouveau toutes les RESCAN secondes tant qu'il manque ; status dit ce qui manque.
- LogWatcher.poll() lit seulement les lignes ajoutées depuis le dernier appel et les donne au parser.
  Si le jeu est relancé (nouveau dossier) ou si le fichier est recréé, on repart de zéro.
"""
import os
import time
from pathlib import Path

from . import hslogs
from .powerlog import PowerLogParser

DEFAULT_LOGS_DIR = Path(r"C:\Program Files (x86)\Hearthstone\Logs")
RESCAN = 5.0                    # secondes entre deux recherches de dossier (pas 4 fois par seconde)
CHUNK = 1 << 20                 # lecture par blocs d'1 Mo : un gros Power.log ne prend pas toute la mémoire d'un coup


def find_power_log(logs_dir=DEFAULT_LOGS_DIR):
    """Power.log du dernier lancement du jeu, ou None si aucun."""
    folders = sorted(Path(logs_dir).glob("Hearthstone_*"), key=lambda p: p.stat().st_mtime)
    for folder in reversed(folders):            # du plus récent au plus ancien
        log = folder / "Power.log"
        if log.exists():
            return log
    return None


class LogWatcher:
    """logs_dir=None : trouvé tout seul (hslogs), y compris si le jeu est installé ailleurs ou lancé après."""

    def __init__(self, logs_dir=None, on_new_game=None, clock=time.monotonic):
        self.fixed_dir = Path(logs_dir) if logs_dir else None
        self.logs_dir = self.fixed_dir
        self.status = "searching"   # no_folder | no_log | ok
        self._clock = clock
        self._last_scan = None
        self.on_new_game = on_new_game
        self.parser = PowerLogParser(on_new_game=on_new_game, keep_games=False)
        self.path = None        # fichier suivi
        self._pos = 0           # position (en octets) déjà lue dans le fichier
        self._partial = b""     # fin de ligne pas encore terminée par le jeu (ou coupée entre deux blocs)

    def _locate(self):
        """Power.log à suivre. Le dossier et le fichier ne sont recherchés que toutes les RESCAN secondes."""
        now = self._clock()
        if self._last_scan is not None and now - self._last_scan < RESCAN:
            return self.path
        self._last_scan = now
        if self.fixed_dir is None:
            self.logs_dir = hslogs.find_logs_dir()
        if self.logs_dir is None:
            self.status = "no_folder"
            return None
        path = find_power_log(self.logs_dir)
        self.status = "ok" if path else "no_log"
        return path

    def poll(self):
        """Lit les nouvelles lignes (1 Mo au plus par appel). Renvoie True si quelque chose a été lu."""
        path = self._locate()
        if path is None:
            return False
        try:
            if path != self.path or os.path.getsize(path) < self._pos:
                # Nouveau fichier (jeu relancé) ou fichier recréé plus petit : on relit depuis le début.
                self.path, self._pos, self._partial = path, 0, b""
                self.parser = PowerLogParser(on_new_game=self.on_new_game, keep_games=False)
            with open(path, "rb") as f:         # binaire : la position en octets reste exacte
                f.seek(self._pos)
                data = f.read(CHUNK)
                self._pos = f.tell()
        except OSError:                         # fichier supprimé entre-temps : on recherchera
            self._last_scan = None
            return False
        if not data:
            return False
        lines = (self._partial + data).split(b"\n")
        self._partial = lines.pop()             # le dernier morceau peut être une ligne inachevée
        for line in lines:                      # découpé en octets : un caractère coupé en deux reste entier
            self.parser.feed_line(line.decode("utf-8", errors="replace"))
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
