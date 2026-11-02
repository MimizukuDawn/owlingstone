"""Fenêtre "toujours au-dessus" : liste des serviteurs par tier, filtrée par les races de la partie.
Elle pilote aussi le panneau Lobby (hud.py, à droite du jeu) et les mises à jour (update.py).

Qui fait quoi :
- LogWorker (un fil d'exécution séparé, QThread) lit Power.log et envoie un "instantané" (snapshot :
  un simple dict) à chaque changement. Ainsi la fenêtre ne gèle jamais, même sur un gros log.
- MainWindow affiche l'instantané : en-tête (héros, tier, tour, PV, or), races, boutons de tier 1-7,
  puis les sections de cartes (comme le navigateur de serviteurs de HDT).
- Couleurs des lignes : or = dans la boutique, vert = sur mon plateau, bleu = dans ma main.
- Masquage automatique (comme HDT, qui disparaît à l'Alt+Tab) : activé par défaut, décochable.
  La fenêtre reste visible si c'est elle qui a le focus (pour pouvoir cliquer sur les tiers).
"""
from collections import Counter
import ctypes
import html
import json
import os
import sys
import threading
import time
from pathlib import Path

from PySide6.QtCore import QDir, QLockFile, QThread, QTimer, Qt, Signal
from PySide6.QtGui import QAction, QIcon, QPainter, QPixmap
from PySide6.QtWidgets import (QApplication, QCheckBox, QComboBox, QDialog, QHBoxLayout, QLabel,
                               QMenu, QMessageBox, QPlainTextEdit, QPushButton, QScrollArea, QSystemTrayIcon, QVBoxLayout,
                               QWidget)

from . import __version__, share
from .advice import (HEROES_PATH, PLAYING_MIN_CONFIDENCE, ShopPick, load_heroes, non_golden, rank_heroes,
                     shop_advice, target_comps)
from .cards import get_db
from .comps import COMPS_PATH, cards_from_ids, hero_best_comps, load_comps, lobby_comps, match_comps
from .history import add_records, format_report, game_record, import_logs, load_history, log_date
from .combat import combat_info
from .hud import HUD_WIDTH, CombatOverlay, LobbyWindow, top_overlay_geometry
from .paths import USER_DIR
from .planner import fight_plan, load_hero_curves, turn_plan
from .trinkets import next_offer_hint, rank_offer
from .pool import (RACE_LABEL, available_keywords, learn_subsets, load_subsets, save_subsets,
                   tier7_reachable, tier_list)
from .powerlog import PowerLogParser
from .rules import RULES_PATH, load_rules
from .update import newer_version, start_update, update_script, whats_new
from .tiles import CARD_URLS, CARDS_DIR, TileCache
from .watcher import DEFAULT_LOGS_DIR, LogWatcher
from .widgets import card_tile, install_preview

SETTINGS_PATH = USER_DIR / "settings.json"
ICON_PATH = Path(__file__).resolve().parent / "icon.ico"

STYLE = """
QWidget { background: #1b1d23; color: #e6e6e6; font-family: Segoe UI; font-size: 12px; }
QPushButton { background: #2a2d36; border: 1px solid #3a3e4a; border-radius: 4px; padding: 4px 0; }
QPushButton:checked { background: #c9a227; color: #111; font-weight: bold; }
QPushButton[current="true"] { border: 2px solid #c9a227; }
QPushButton[far="true"] { color: #5c606b; }
QComboBox { background: #2a2d36; border: 1px solid #3a3e4a; border-radius: 4px; padding: 2px 6px; }
QScrollArea { border: none; }
QLabel#section { color: #c9a227; font-weight: bold; padding-top: 6px; }
#row { border-radius: 3px; }
#row QLabel { background: transparent; padding: 2px 6px; }
#row[state="shop"]  { background: #4a3d12; }
#row[state="board"] { background: #1f4a2a; }
#row[state="hand"]  { background: #1d3550; }
QLabel#status { color: #9aa0ad; }
#comp { background: #20232b; border: 1px solid #2c2f39; border-radius: 6px; }
#comp[state="playing"] { border: 2px solid #e0b830; }
#comp[state="open"] { border: 1px solid #9aa0ad; }
#comp QLabel { background: transparent; }
QLabel#section { color: #c9a227; font-weight: bold; padding-top: 6px; letter-spacing: 1px; }
#prow { border-radius: 3px; }
#prow QLabel { background: transparent; }
#prow[state="me"]   { background: #1f4a2a; }
#prow[state="next"] { background: #4a3d12; }
#prow[state="dead"] QLabel { color: #5c606b; }
#prow[selected="true"] { border: 1px solid #c9a227; }
#prow:hover { background: #2a2d36; }
QPushButton#update { background: #c9a227; color: #111; font-weight: bold; padding: 4px 8px; }
"""


# ------------------------------------------------------------------ réglages persistants
def load_settings():
    try:
        return json.loads(SETTINGS_PATH.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def save_settings(settings):
    SETTINGS_PATH.parent.mkdir(parents=True, exist_ok=True)
    SETTINGS_PATH.write_text(json.dumps(settings, indent=1), encoding="utf-8")


# ------------------------------------------------------------------ instantané de la partie
LOBBY_SHOWN = 6   # comps du lobby affichées en début de partie (les meilleures d'abord)


def make_snapshot(game):
    """Copie simple (dict) de ce que la fenêtre affiche : pas d'objet partagé entre les 2 fils."""
    if game is None or not game.is_battlegrounds:
        return {"in_game": False}
    hero = game.my_hero

    def ids(entities):
        return [get_db().base(e.card_id).id for e in entities if e.card_id and get_db().base(e.card_id)]

    mine = game.my_board + game.my_hand

    return {
        "in_game": True,
        "hero": hero.name if hero else None,
        "tier": game.tavern_tier,
        "turn": game.turn,
        "health": game.my_health,
        "gold": game.my_gold,
        "races": sorted(game.available_races) if game.shop_race_counts else None,
        "races_complete": game.races_complete,
        "possible_races": game.possible_races,
        "deity": game.deity,
        "duos": game.is_duos,
        "hero_options": [e.name for e in game.hero_options],
        "hero_option_ids": [e.card_id for e in game.hero_options],
        "board": ids(game.my_board),
        "hand": ids(game.my_hand),
        "shop": ids(game.shop),
        "golden": ids([e for e in mine if e.is_golden]),   # dorés : ne comptent pas pour un triple
        "over": game.is_over,
        "placement": game.my_placement,
        "players": lobby_players(game),
        "bonuses": game.my_bonuses,
        "hero_powers": [(e.name, e.card_id) for e in game.my_hero_powers],
        "fights": [{"turn": f["turn"], "opponent": f["opponent"], "result": f["result"], "damage": f["damage"]}
                   for f in game.fights],
        "trinket_offer": [(e.card_id, cost) for e, cost in game.trinket_offer],   # Trinkets à choisir (tours 6 et 9)
        "trinkets": [e.card_id for e in game.my_trinkets],
        "combat": combat_info(game),          # chances de gagner le combat (combat.py)
        "plan": fight_plan(game),             # simulations du tour : acheter, monter... (planner.py)
    }


def lobby_players(game):
    """Les 8 joueurs pour le panneau Lobby : héros, place, PV, tier, triples, dernier combat contre moi."""
    me, nxt = game.my_player_id, game.next_opponent_id
    players = []
    for h in game.lobby_heroes:              # (une seule lecture des entités : appelé 2 fois par seconde)
        pid = h.get("PLAYER_ID")
        fight = game.last_fight_against(pid) if pid != me else None
        health = h.get("HEALTH", 0) - h.get("DAMAGE", 0) + h.get("ARMOR", 0)
        players.append({
            "id": pid, "hero": h.name, "hero_id": h.card_id, "place": h.get("PLAYER_LEADERBOARD_PLACE"),
            "health": health, "tier": h.get("PLAYER_TECH_LEVEL"), "triples": h.get("PLAYER_TRIPLES", 0),
            "dead": health is not None and health <= 0, "me": pid == me, "next": pid == nxt and pid != me,
            "fight": {"turn": fight["turn"], "result": fight["result"], "damage": fight["damage"],
                      "board": [dict(m, card_id=_base_id(m["card_id"])) for m in fight["board"]]} if fight else None,
        })
    return players


def _base_id(card_id):
    """Id de la version normale (pas dorée) d'une carte, pour la liste et les images."""
    c = get_db().base(card_id) if card_id else None
    return c.id if c else card_id


class LogWorker(QThread):
    snapshot = Signal(dict)   # signal Qt : transporte l'instantané vers la fenêtre

    def __init__(self, replay_path=None, stop_turn=None):
        super().__init__()
        self.replay_path = replay_path
        self.stop_turn = stop_turn

    def run(self):
        if self.replay_path:
            self._run_replay()
        else:
            self._run_live()

    def _run_replay(self):
        """Relit un vieux log jusqu'au tour demandé (pour tester la fenêtre hors partie)."""
        parser = PowerLogParser()
        with open(self.replay_path, encoding="utf-8", errors="replace") as f:
            for line in f:
                parser.feed_line(line)
                g = parser.game
                if self.stop_turn and g and g.turn >= self.stop_turn and g.shop and len(g.shop) >= 3:
                    break
                if self.stop_turn == 0 and g and len(g.hero_options) >= 4:   # --turn 0 : choix du héros
                    break
        self.snapshot.emit(make_snapshot(parser.game))

    def _run_live(self):
        watcher = LogWatcher()
        last, learned_for = None, None
        while not self.isInterruptionRequested():
            if watcher.poll():
                g = watcher.game
                snap = make_snapshot(g)
                if snap != last:
                    last = snap
                    self.snapshot.emit(snap)
                # Fin de partie : on retient les neutres liés à une race (tags BACON_SUBSET_*)
                # et on note la partie dans l'historique (history.jsonl).
                if g is not None and g.is_over and learned_for is not g:
                    learned_for = g
                    save_subsets(learn_subsets([g], load_subsets()))
                    try:
                        record = game_record(g, load_comps(), log_date(watcher.path))
                        add_records([record])
                        if share.add_game(g, record):     # seulement si le joueur a dit oui
                            send_in_background()
                    except (OSError, ValueError):
                        pass                    # comps.yaml cassé : la fenêtre l'affiche déjà
            time.sleep(0.25)            # 4 lectures par seconde : un changement (pouvoir, boutique) se voit vite


# ------------------------------------------------------------------ détection du premier plan
def foreground_exe():
    """Nom de l'exécutable de la fenêtre active (Windows), ex. 'hearthstone.exe'."""
    try:
        user32, kernel32 = ctypes.windll.user32, ctypes.windll.kernel32
        hwnd = user32.GetForegroundWindow()
        pid = ctypes.c_ulong()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        handle = kernel32.OpenProcess(0x1000, False, pid.value)  # 0x1000 = lecture limitée
        buf = ctypes.create_unicode_buffer(260)
        size = ctypes.c_ulong(260)
        kernel32.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size))
        kernel32.CloseHandle(handle)
        return Path(buf.value).name.lower()
    except (AttributeError, OSError):
        return ""


def hearthstone_rect():
    """Rectangle écran (x, y, largeur, hauteur) de la zone de jeu de Hearthstone, ou None."""
    try:
        user32 = ctypes.windll.user32
        hwnd = user32.FindWindowW("UnityWndClass", "Hearthstone")  # classe de fenêtre Unity
        if not hwnd or user32.IsIconic(hwnd):                      # absente ou réduite
            return None

        class RECT(ctypes.Structure):
            _fields_ = [("left", ctypes.c_long), ("top", ctypes.c_long),
                        ("right", ctypes.c_long), ("bottom", ctypes.c_long)]

        class POINT(ctypes.Structure):
            _fields_ = [("x", ctypes.c_long), ("y", ctypes.c_long)]

        rc, origin = RECT(), POINT(0, 0)
        user32.GetClientRect(hwnd, ctypes.byref(rc))          # taille de la zone de jeu
        user32.ClientToScreen(hwnd, ctypes.byref(origin))     # son coin haut-gauche à l'écran
        return origin.x, origin.y, rc.right, rc.bottom
    except (AttributeError, OSError):
        return None


class StatsDialog(QDialog):
    """Mes statistiques (history.jsonl) : place moyenne par comp, Déité, héros, et mes règles."""

    def __init__(self, comps, rules, parent=None):
        super().__init__(parent)
        self.comps, self.rules = comps, rules
        self.setWindowTitle("Owlingstone – my stats")
        self.resize(640, 560)
        lay = QVBoxLayout(self)
        self.text = QPlainTextEdit()
        self.text.setReadOnly(True)
        self.text.setStyleSheet("font-family: Consolas, monospace;")
        lay.addWidget(self.text)
        row = QHBoxLayout()
        imp = QPushButton("Import old games from Hearthstone logs")
        imp.clicked.connect(self.import_logs)
        row.addWidget(imp)
        row.addStretch(1)
        close = QPushButton("Close")
        close.clicked.connect(self.accept)
        row.addWidget(close)
        lay.addLayout(row)
        self.refresh()

    def refresh(self, prefix=""):
        self.text.setPlainText(prefix + format_report(load_history(), self.rules))

    def import_logs(self):
        paths = sorted(DEFAULT_LOGS_DIR.glob("Hearthstone_*/Power*.log"))
        QApplication.setOverrideCursor(Qt.WaitCursor)     # peut prendre quelques secondes
        try:
            added = add_records(import_logs(paths, self.comps))
        finally:
            QApplication.restoreOverrideCursor()
        self.refresh(f"{len(paths)} log files read, {added} new games added.\n\n")


# ------------------------------------------------------------------ fenêtre
PANEL_MIN_WIDTH, PANEL_STEP = 170, 20


def side_panel_geometry(rect, side, delta=0):
    """Place d'un panneau collé au bord gauche ou droit du jeu, dans la bande de décor qui entoure la
    zone 4:3 (le classement des héros est juste à l'intérieur de cette zone à gauche, le plateau au
    centre). La fenêtre gauche prend ~80 % de la bande, le panneau droit la bande moins une marge.
    delta = réglage de l'utilisateur (menu de l'icône : plus étroit / plus large)."""
    x, y, w, h = rect
    free = (w - h * 4 / 3) / 2              # largeur du décor d'un côté de la zone 4:3
    if side == "left":
        width = min(320, free * 0.8)
    else:
        width = min(HUD_WIDTH, free - 10)
    width = int(max(PANEL_MIN_WIDTH, width + delta))
    left = x if side == "left" else x + w - width
    return left, y + int(h * 0.06), width, int(h * 0.85)


class MainWindow(QWidget):
    def __init__(self, replay_mode=False):
        super().__init__()
        self.replay_mode = replay_mode
        self.settings = load_settings()
        self.snap = {"in_game": False}
        self.manual_tier = None          # None = suit mon tier de taverne ; sinon tier choisi au clic
        self._last_render_key = None
        self.seen_tier7 = False          # une carte de tier 7 est apparue dans cette partie
        self._keyword_races = "?"        # races pour lesquelles la liste de mots-clés a été remplie

        self.setWindowTitle("Owlingstone")
        # Comme l'overlay de HDT : sans cadre, toujours au-dessus, absent de la barre des tâches (Tool).
        self.setWindowFlags(Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.Tool)
        self.setWindowOpacity(0.93)
        self.setStyleSheet(STYLE)
        self.resize(340, 720)

        root = QVBoxLayout(self)
        root.setContentsMargins(8, 8, 8, 8)
        self.header = QLabel()
        self.header.setTextFormat(Qt.RichText)
        self.races = QLabel()
        self.races.setTextFormat(Qt.RichText)
        self.races.setWordWrap(True)
        root.addWidget(self.header)
        root.addWidget(self.races)
        # Panneau "ce que je joue" : comp détectée, cartes possédées, cartes clés qui manquent.
        self.comp_label = QLabel()
        self.comp_label.setTextFormat(Qt.RichText)
        self.comp_label.setWordWrap(True)
        root.addWidget(self.comp_label)
        self.load_data()
        self.comp_html = ""
        self.advice = {}                      # même texte, repris dans l'onglet Comps du panneau de droite
        self.playing_comp = None                 # id de la comp que je joue (reconnue sur mon plateau)
        self.comp_cards = set()                  # ids des cartes de la comp détectée (★ dans la liste)
        self.buy_cards = set()                   # ids des achats conseillés (▶ dans la liste)

        tiers = QHBoxLayout()
        self.tier_buttons = []
        for t in range(1, 8):
            b = QPushButton(str(t))
            b.setCheckable(True)
            b.clicked.connect(lambda _=False, t=t: self.on_tier_clicked(t))
            tiers.addWidget(b)
            self.tier_buttons.append(b)
        root.addLayout(tiers)
        # Filtre par mot-clé (Deathrattle, Battlecry...) : seulement ceux possibles dans ce lobby.
        self.keyword_box = QComboBox()
        self.keyword_box.currentIndexChanged.connect(lambda _: self.render())
        root.addWidget(self.keyword_box)

        self.scroll = QScrollArea()
        self.scroll.setWidgetResizable(True)
        root.addWidget(self.scroll, 1)

        self.hide_box = QCheckBox("Auto-hide when Hearthstone is not focused")
        # Activé par défaut (par défaut) : comme HDT, la fenêtre disparaît à l'Alt+Tab.
        self.hide_box.setChecked(self.settings.get("hide_when_unfocused", True))
        self.hide_box.toggled.connect(self.on_hide_toggled)
        root.addWidget(self.hide_box)
        bottom = QHBoxLayout()
        self.art_box = QCheckBox("Card art")
        self.art_box.setChecked(self.settings.get("card_art", True))
        self.art_box.toggled.connect(self.on_art_toggled)
        bottom.addWidget(self.art_box)
        bottom.addStretch(1)
        self.lobby_box = QCheckBox("Right panel")
        self.lobby_box.setToolTip("Panel on the right of the game. Comps tab: what I play, what to buy, every comp "
                                  "of this lobby. Lobby tab: next opponent's last board, the 8 players.")
        self.lobby_box.setChecked(self.settings.get("lobby_panel", True))
        self.lobby_box.toggled.connect(self.on_lobby_toggled)
        bottom.addWidget(self.lobby_box)
        stats = QPushButton("My stats")
        stats.clicked.connect(self.show_stats)
        bottom.addWidget(stats)
        root.addLayout(bottom)
        # Ligne du bas : version, et bouton de mise à jour quand une nouvelle version est prête.
        version_row = QHBoxLayout()
        self.version_label = QLabel(f"v{__version__}")
        self.version_label.setObjectName("status")
        version_row.addWidget(self.version_label)
        version_row.addStretch(1)
        self.update_button = QPushButton()
        self.update_button.setObjectName("update")
        self.update_button.clicked.connect(self.run_update)
        self.update_button.hide()
        version_row.addWidget(self.update_button)
        root.addLayout(version_row)
        self.tray = None                         # icône de notification (posée par run())
        self.update_ready = None                 # numéro de la nouvelle version prête, sinon None

        # Images des cartes : téléchargées en arrière-plan, la liste est redessinée quand elles arrivent
        # (regroupé : une seule mise à jour pour une rafale d'images).
        self.tiles = TileCache(parent=self)
        # Image entière de la carte survolée (comme HDT), téléchargée elle aussi à la demande.
        self.preview = install_preview(TileCache(CARDS_DIR, self, urls=CARD_URLS))
        self.tile_timer = QTimer(self)
        self.tile_timer.setSingleShot(True)
        self.tile_timer.timeout.connect(self.redraw_cards)
        self.tiles.ready.connect(lambda _id: self.tile_timer.start(150))

        # Vérifie 2 fois par seconde si Hearthstone est au premier plan (option "cacher").
        self.focus_timer = QTimer(self)
        self.focus_timer.timeout.connect(self.check_focus)
        self.focus_timer.start(500)
        # Panneau Lobby, à droite du jeu.
        self.hud = LobbyWindow(self)
        # Chances du combat en haut du jeu, au-dessus de l'adversaire (comme HDT).
        self.top_odds = CombatOverlay()
        self._has_odds = False
        # Toutes les 3 s : comps.yaml / rules.yaml / heroes.yaml modifiés ? On les recharge.
        self.data_timer = QTimer(self)
        self.data_timer.timeout.connect(self.check_data_files)
        self.data_timer.start(3000)
        # Toutes les minutes : une nouvelle version est-elle prête dans le projet ?
        self.update_timer = QTimer(self)
        self.update_timer.timeout.connect(self.check_update)
        self.update_timer.start(60_000)
        QTimer.singleShot(2000, self.check_update)
        self.render()

    # -------------------------------------------------------------- données et mises à jour
    DATA_FILES = (COMPS_PATH, RULES_PATH, HEROES_PATH)

    def data_mtimes(self):
        stamps = []
        for path in self.DATA_FILES:
            try:
                stamps.append(path.stat().st_mtime)
            except OSError:
                stamps.append(None)
        return stamps

    def load_data(self):
        self._data_stamps = self.data_mtimes()
        try:
            self.comps = load_comps()
            self.rules = load_rules(comp_ids={c.id for c in self.comps})
            self.heroes = load_heroes()
            self.comps_error = None
        except (OSError, ValueError) as e:      # fichier absent ou faute de frappe : on l'affiche
            self.comps, self.rules, self.heroes, self.comps_error = [], [], {}, str(e)

    def check_data_files(self):
        """Un fichier de données a été enregistré : on le relit (pas besoin de relancer l'appli)."""
        if self.data_mtimes() != self._data_stamps:
            self.load_data()
            self.redraw_cards()
            self.hud.render(force=True)

    def check_update(self):
        found = newer_version()
        if found and found != self.update_ready:
            self.update_ready = found
            self.update_button.setText(f"Update to v{found}")
            self.update_button.setToolTip("Closes Owlingstone, installs the new version and restarts it. "
                                          "Your history and settings are kept.")
            self.update_button.show()
            if self.tray:
                self.tray.update_action.setText(f"Update to v{found}")
                self.tray.showMessage("Owlingstone", f"Version {found} is ready. Click \"Update to v{found}\" "
                                      "in the window or the tray menu.", QSystemTrayIcon.Information, 8000)

    def run_update(self):
        if start_update():
            QApplication.quit()               # update.bat ferme aussi l'exe s'il tourne encore
        elif self.tray:
            self.tray.showMessage("Owlingstone", "The update could not start. Try again later.",
                                  QSystemTrayIcon.Warning, 6000)

    # -------------------------------------------------------------- événements
    def on_snapshot(self, snap):
        if snap.get("tier") != self.snap.get("tier"):
            self.manual_tier = None      # on monte de tier : la liste suit à nouveau
        if not snap.get("in_game") or (snap.get("turn") or 0) < (self.snap.get("turn") or 0):
            self.seen_tier7 = False      # nouvelle partie
        mine = cards_from_ids(snap.get("shop", []) + snap.get("board", []) + snap.get("hand", []))
        self.seen_tier7 = self.seen_tier7 or any(c.tier == 7 for c in mine)
        self.snap = snap
        self.render()
        self.hud.on_snapshot(snap)
        self._has_odds = bool(snap.get("in_game")) and self.top_odds.show_combat(snap.get("combat"))

    def on_tier_clicked(self, tier):
        current = self.snap.get("tier")
        # Cliquer sur son propre tier (ou re-cliquer le tier choisi) = revenir au suivi automatique.
        self.manual_tier = None if tier in (current, self.manual_tier) else tier
        self.render()

    def on_art_toggled(self, checked):
        self.settings["card_art"] = checked
        save_settings(self.settings)
        self.redraw_cards()
        self.hud.render(force=True)

    def on_lobby_toggled(self, checked):
        self.settings["lobby_panel"] = checked
        save_settings(self.settings)
        self.place_hud()
        self.render()

    def save_settings(self):
        save_settings(self.settings)

    def redraw_cards(self):
        self._last_render_key = None         # force la reconstruction de la liste
        self.render()

    def show_stats(self):
        StatsDialog(self.comps, self.rules, self).exec()

    def on_hide_toggled(self, checked):
        self.settings["hide_when_unfocused"] = checked
        save_settings(self.settings)
        if not checked:
            self.show()

    def check_focus(self):
        """Toutes les 0,5 s : se coller à Hearthstone, et se montrer/cacher comme HDT."""
        if self.replay_mode:
            self.place_hud()
            return                      # relecture d'un vieux log : fenêtre libre, toujours visible
        rect = hearthstone_rect()
        self._game_rect = rect
        if rect:
            x, y, w, h = rect
            self.setGeometry(*side_panel_geometry(rect, "left", self.settings.get("panel_delta", 0)))
        if not self.hide_box.isChecked():
            if not self.isVisible():
                self.show()
        else:
            exe = foreground_exe()
            focused = exe in ("hearthstone.exe", Path(sys.executable).name.lower())
            # Visible seulement : jeu (ou cette fenêtre) au premier plan ET partie Battlegrounds en cours.
            visible = focused and self.snap.get("in_game") and rect is not None
            if visible and not self.isVisible():
                self.show()
            elif not visible and self.isVisible():
                self.hide()
                self.preview.hide()
        self.place_hud()

    def resize_panels(self, step):
        """Menu de l'icône : les deux panneaux plus étroits (step < 0) ou plus larges, retenu."""
        delta = max(-160, min(160, self.settings.get("panel_delta", 0) + step))
        self.settings["panel_delta"] = delta
        save_settings(self.settings)

    def place_hud(self):
        """Le panneau Lobby suit la fenêtre principale : visible avec elle (si la case est cochée),
        collé au bord droit du jeu (ou juste à droite de la fenêtre, hors partie / en relecture)."""
        want = self.isVisible() and self.lobby_box.isChecked()
        rect = getattr(self, "_game_rect", None)
        if want:
            if rect and not self.replay_mode:
                x, y, w, h = rect
                gx, gy, gw, gh = side_panel_geometry(rect, "right", self.settings.get("panel_delta", 0))
                self.hud.setGeometry(gx, gy, gw, gh)
                # Si Qt impose une taille plus grande (contenu trop large), on reste collé au bord droit
                # du jeu au lieu de déborder sur l'écran voisin.
                if self.hud.x() + self.hud.width() > x + w:
                    self.hud.move(x + w - self.hud.width(), gy)
            elif not self.hud.isVisible():
                self.hud.setGeometry(self.x() + self.width() + 10, self.y(), HUD_WIDTH, self.height())
        if want != self.hud.isVisible():
            self.hud.setVisible(want)
        top = (self.isVisible() and rect is not None and not self.replay_mode and self._has_odds
               and self.settings.get("top_odds", True))
        if top:
            self.top_odds.setGeometry(*top_overlay_geometry(rect))
        if top != self.top_odds.isVisible():
            self.top_odds.setVisible(top)

    # -------------------------------------------------------------- affichage
    def shown_tier(self):
        return self.manual_tier or self.snap.get("tier") or 1

    def render(self):
        s = self.snap
        tier = self.shown_tier()
        # En-tête
        if not s.get("in_game"):
            self.header.setText("<span style='color:#9aa0ad'>Waiting for a Battlegrounds game…</span>")
        else:
            parts = [f"<b>{html.escape(s['hero'] or 'Choosing hero…')}</b>",
                     f"Tier {s['tier'] or '?'}", f"Turn {s['turn']}"]
            if s.get("health") is not None:
                parts.append(f"❤ {s['health']}")
            if s.get("gold") is not None:
                parts.append(f"🪙 {s['gold']}")
            if s.get("over"):
                parts.append(f"<b>Finished #{s['placement']}</b>")
            self.header.setText(" · ".join(parts))
        # Races
        races = s.get("races")
        if not races:
            self.races.setText("<span style='color:#9aa0ad'>Races: detecting from Bob's shop…</span>")
        else:
            chips = " ".join(f"<span style='background:#2f5d3a;padding:2px'>&nbsp;{RACE_LABEL[r]}&nbsp;</span>"
                             for r in races)
            missing = 5 - len(races)
            extra = ""
            if missing > 0:
                maybe = ", ".join(RACE_LABEL[r] for r in s.get("possible_races") or [])
                extra = f" <span style='color:#9aa0ad'>+{missing} unknown{(' (maybe ' + maybe + ')') if maybe else ''}</span>"
            self.races.setText(chips + extra)
        self.render_comp()
        # Boutons de tier (le 7 grisé tant qu'il n'est pas atteignable dans ce lobby, mais cliquable)
        sure = races if s.get("races_complete") else None
        t7 = tier7_reachable(sure, duos=s.get("duos", False), seen_tier7=self.seen_tier7 or (s.get("tier") or 0) >= 7)
        for b in self.tier_buttons:
            t = int(b.text())
            b.setChecked(t == tier)
            b.setProperty("current", t == s.get("tier"))
            b.setProperty("far", t == 7 and not t7)
            b.setToolTip("" if t != 7 or t7 else "Tier 7 not reachable yet: no Greedy Conniver (Pirates) in this "
                         "lobby, and no Tier 7 card seen. Click to look anyway.")
            b.style().unpolish(b)
            b.style().polish(b)
        keyword = self.update_keywords(races, s.get("duos", False))
        # Cartes : on ne reconstruit que si quelque chose a changé
        key = (tier, tuple(races or ()), keyword, tuple(s.get("shop", [])), tuple(s.get("board", [])),
               tuple(s.get("hand", [])), tuple(sorted(self.comp_cards)), tuple(sorted(self.buy_cards)))
        if key != self._last_render_key:
            self._last_render_key = key
            self.scroll.setWidget(self.build_cards(tier, races, keyword))

    def update_keywords(self, races, duos):
        """Remplit la liste des mots-clés quand les races changent ; renvoie le mot-clé choisi (ou None)."""
        if self._keyword_races != (tuple(races or ()), duos):
            self._keyword_races = (tuple(races or ()), duos)
            chosen = self.keyword_box.currentData()
            self.keyword_box.blockSignals(True)      # pas de render() pendant qu'on la remplit
            self.keyword_box.clear()
            self.keyword_box.addItem("All keywords", None)
            for label in available_keywords(races, duos=duos):
                self.keyword_box.addItem(label, label)
            self.keyword_box.setCurrentIndex(max(0, self.keyword_box.findData(chosen)))
            self.keyword_box.blockSignals(False)
        return self.keyword_box.currentData()

    def render_comp(self):
        """Texte comp (ici, ou dans l'onglet Comps à droite si le panneau est actif) et cartes ★."""
        self.playing_comp = None
        self._render_comp_text()
        self.comp_html = self.comp_label.text()
        # Panneau de droite actif : le texte n'y est montré qu'une fois, la liste des tiers garde la place.
        self.comp_label.setVisible(not self.lobby_box.isChecked() or bool(self.comps_error))
        hud = getattr(self, "hud", None)
        opened = hud.shown_comp() if hud else self.playing_comp
        if opened != self.playing_comp:      # comp ouverte à droite : c'est elle qu'on marque ★
            comp = next((c for c in self.comps if c.id == opened), None)
            self.comp_cards = {c.id for c in comp.core + comp.addons} if comp else set()

    def _render_comp_text(self):
        """Texte du panneau comp : la meilleure comp de mon plateau, ou les comps du lobby."""
        s = self.snap
        if self.comps_error:
            self.comp_label.setText(f"<span style='color:#e57373'>{html.escape(self.comps_error)}</span>")
            return
        self.comp_cards, self.buy_cards = set(), set()
        # Même contenu en données, pour le panneau de droite qui l'affiche en images (hud.py).
        self.advice = {"deity": s.get("deity"), "heroes": [], "playing": None, "alt": None, "buy": [], "plan": None,
                       "trinkets": None}
        if not s.get("in_game"):
            self.comp_label.setText("")
            return
        deity = f" · Deity: <b>{html.escape(s['deity'])}</b>" if s.get("deity") else ""
        if s.get("hero_options"):
            ids = dict(zip(s["hero_options"], s.get("hero_option_ids", [])))
            self.advice["heroes"] = [(name, ids.get(name), st) for name, st in rank_heroes(s["hero_options"], self.heroes)]
            # Comps où chaque héros proposé fait partie des meilleurs (bgbuddy) : toutes, les races du lobby
            # n'étant pas encore dans le log au moment du choix
            races = s.get("races") if s.get("races_complete") else None
            self.advice["hero_comps"] = {name: hero_best_comps(name, self.comps, races, top=2)
                                         for name in s["hero_options"]}
            curves = load_hero_curves()      # courbe / pouvoir propres au héros (data/hero_curves.yaml)
            self.advice["hero_notes"] = {name: (curves.get(name) or {}).get("note", "") for name in s["hero_options"]}
            self.comp_label.setText(self.hero_text(s["hero_options"]) + deity)
            return
        races = s.get("races") if s.get("races_complete") else None
        board, hand = cards_from_ids(s.get("board", [])), cards_from_ids(s.get("hand", []))
        game = {"rules": self.rules, "deity": s.get("deity"), "hero": s.get("hero"),
                "hero_power": [name for name, _ in s.get("hero_powers") or []] or None}
        self.advice["trinkets"] = self.trinket_advice(board, hand, races, game)
        buy = self.buy_text(board, hand, races, game)
        matches = match_comps(self.comps, board, hand, races, top=2, min_confidence=PLAYING_MIN_CONFIDENCE, **game)
        if not matches:
            # Début de partie : on liste les comps jouables dans ce lobby.
            ranked = lobby_comps(self.comps, races, **game)
            names = ", ".join(c.name for c in ranked[:LOBBY_SHOWN])
            if len(ranked) > LOBBY_SHOWN:
                names += f" (+{len(ranked) - LOBBY_SHOWN} more)"
            self.comp_label.setText(f"<span style='color:#9aa0ad'>Lobby comps:</span> {html.escape(names)}{deity}{buy}")
            return
        best = matches[0]
        self.advice["playing"], self.advice["alt"] = best, (matches[1] if len(matches) > 1 else None)
        self.playing_comp = best.comp.id
        self.comp_cards = {c.id for c in best.comp.core + best.comp.addons}
        need = ", ".join(f"{html.escape(c.name)} (T{c.tier})" for c in best.missing_core[:3])
        alt = (f"<br><span style='color:#9aa0ad'>or {html.escape(matches[1].comp.name)} "
               f"({matches[1].confidence}%)</span>") if len(matches) > 1 else ""
        self.comp_label.setText(
            f"Playing: <b style='color:#c9a227'>{html.escape(best.comp.name)}</b> ({best.confidence}%){deity}<br>"
            f"<span style='color:#9aa0ad'>Have:</span> {html.escape(', '.join(c.name for c in best.have)) or '–'}<br>"
            f"<span style='color:#9aa0ad'>Need:</span> {need or 'all core cards ✓'}{alt}{buy}")
        self.comp_label.setToolTip("\n".join([best.comp.when_to_commit, best.comp.how_to_play] + best.notes))

    def hero_text(self, options):
        """Héros proposés, du meilleur placement moyen au moins bon (le premier en or)."""
        parts = []
        for i, (name, st) in enumerate(rank_heroes(options, self.heroes)):
            label = html.escape(name) + (f" <span style='color:#9aa0ad'>{st['avg']:.2f}</span>" if st else "")
            parts.append(f"<b style='color:#c9a227'>{label}</b>" if i == 0 and st else label)
        return "<span style='color:#9aa0ad'>Pick (avg place):</span> " + " · ".join(parts)

    def buy_text(self, board, hand, races, game):
        """Ligne "Buy:" : les 3 meilleurs achats de la boutique (vide hors recrutement)."""
        s = self.snap
        shop = cards_from_ids(s.get("shop", []))
        if not shop or s.get("over"):
            return ""
        owned = non_golden(board + hand, s.get("golden", []))
        targets = target_comps(self.comps, board, hand, races, **game)
        picks = shop_advice(shop, owned, targets)
        plan = s.get("plan")
        if plan:
            # Plan du tour (planner.py) : les points comp + ce que chaque carte change au prochain combat
            have = {c.id for c in owned}
            copies = Counter(c.id for c in owned)
            wanted = [c for comp, w in targets if w >= 0.5 for c in comp.core if c.id not in have]
            wanted += [c for c in owned if copies[c.id] == 2]
            protect = {c.id for comp, w in targets if w >= 0.5 for c in comp.core + comp.addons}
            protect |= {cid for cid, n in copies.items() if n >= 2}
            advice = turn_plan(plan, picks, wanted, self.pool_up_to(s.get("tier") or 1, races), protect)
            advice["fight"] = plan
            advice["hero_best"] = hero_best_comps(s.get("hero"), self.comps, races, top=2)
            self.advice["plan"] = advice
            picks = [ShopPick(b[0], b[2], b[3]) for b in advice["buy"]]
        picks = picks[:3]
        self.buy_cards = {p.card.id for p in picks}
        self.advice["buy"] = picks
        if not picks:
            return ""
        text = ", ".join(f"{html.escape(p.card.name)} <span style='color:#9aa0ad'>({html.escape(', '.join(p.reasons))})</span>"
                         for p in picks)
        return f"<br><span style='color:#7fc97f'>Buy:</span> {text}"

    def trinket_advice(self, board, hand, races, game):
        """Trinkets (trinkets.py) : l'offre en cours classée, ou avant l'offre (tours 5 et 8) ce que mon plateau débloque."""
        s = self.snap
        if s.get("over"):
            return None
        targets = target_comps(self.comps, board, hand, races, **game)
        offer = [(get_db().get(cid), cost) for cid, cost in s.get("trinket_offer") or []]
        offer = [(c, cost) for c, cost in offer if c is not None]
        if offer:
            powers = [get_db().get(cid) for _, cid in s.get("hero_powers") or []]
            picks = rank_offer(offer, board, hand, cards_from_ids(s.get("shop", [])), targets,
                               [p for p in powers if p], s.get("gold"))
            return {"offer": picks, "hint": None}
        hint = next_offer_hint(s.get("turn") or 0, board, hand, races, targets)
        return {"offer": [], "hint": hint} if hint else None

    def pool_up_to(self, tier, races):
        """Serviteurs de ce lobby du tier 1 à mon tier (pour les chances de trouver une carte en relançant)."""
        key = (tier, tuple(races or ()), self.snap.get("duos", False))
        if getattr(self, "_pool_key", None) != key:
            subsets = load_subsets()
            cards = {}
            for t in range(1, tier + 1):
                for section, found in tier_list(t, races, subsets, duos=key[2]):
                    if section not in ("SPELL", "VARIES"):
                        cards.update((c.id, c) for c in found)
            self._pool_key, self._pool_cards = key, list(cards.values())
        return self._pool_cards

    def build_cards(self, tier, races, keyword=None):
        s = self.snap
        shop, board, hand = set(s.get("shop", [])), set(s.get("board", [])), set(s.get("hand", []))
        w = QWidget()
        lay = QVBoxLayout(w)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(2)
        for section, cards in tier_list(tier, races, duos=s.get("duos", False), keyword=keyword):
            title = QLabel(f"{RACE_LABEL.get(section, section)}  ({len(cards)})")
            title.setObjectName("section")
            lay.addWidget(title)
            for c in cards:
                mark = ("▶" if c.id in self.buy_cards else "") + ("★" if c.id in self.comp_cards else "")
                state = "shop" if c.id in shop else "board" if c.id in board else "hand" if c.id in hand else ""
                lay.addWidget(card_tile(c, self.tiles, self.art_box.isChecked(), state=state, mark=mark))
        lay.addStretch(1)
        return w


# ------------------------------------------------------------------ partage (opt-in) et avis
def send_in_background():
    """Envoie la file de partage sans bloquer la fenêtre (un seul envoi groupé)."""
    def run():
        try:
            share.flush()
        except Exception:             # réseau, serveur... la file attend le prochain envoi
            pass
    threading.Thread(target=run, name="share", daemon=True).start()


SHARE_QUESTION = (
    "Share anonymous game summaries to improve everyone's advice?\n\n"
    "When a game ends, Owlingstone would send your hero, placement, comp, lobby tribes, final board and the "
    "boards of each fight (cards and stats only). Never your BattleTag, other players' names, your logs or "
    "files. You can turn it off or delete what you sent at any time from the tray menu.")


def ask_sharing(parent=None):
    """Demandé une seule fois. NON par défaut."""
    if share.asked() or not share.API_URL:
        return
    box = QMessageBox(QMessageBox.Question, "Owlingstone", SHARE_QUESTION, QMessageBox.Yes | QMessageBox.No, parent)
    box.setDefaultButton(QMessageBox.No)
    share.set_enabled(box.exec() == QMessageBox.Yes)


class FeedbackDialog(QDialog):
    """Clic droit sur l'icône > Send feedback : une idée, un bug ou autre, envoyé avec le prochain envoi."""

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Owlingstone – send feedback")
        lay = QVBoxLayout(self)
        lay.addWidget(QLabel("Ideas, bugs, anything: it helps decide the next updates."))
        self.kind = QComboBox()
        self.kind.addItems(["Idea", "Bug", "Other"])
        lay.addWidget(self.kind)
        self.text = QPlainTextEdit()
        self.text.setPlaceholderText("Write here (1000 characters max). Please don't include personal information.")
        self.text.textChanged.connect(self._limit)
        lay.addWidget(self.text)
        self.info = QLabel("Sent anonymously with your next game, or when Owlingstone next starts.")
        self.info.setWordWrap(True)
        lay.addWidget(self.info)
        row = QHBoxLayout()
        cancel, send = QPushButton("Cancel"), QPushButton("Send")
        cancel.clicked.connect(self.reject)
        send.clicked.connect(self.send)
        row.addStretch(1)
        row.addWidget(cancel)
        row.addWidget(send)
        lay.addLayout(row)
        self.resize(420, 300)

    def _limit(self):
        text = self.text.toPlainText()
        if len(text) > 1000:
            self.text.setPlainText(text[:1000])

    def send(self):
        try:
            share.add_feedback(self.kind.currentText().lower(), self.text.toPlainText())
        except ValueError:
            self.info.setText("Please write at least a few words.")
            return
        self.accept()


def show_shared(parent=None):
    try:
        text = share.LAST_PATH.read_text(encoding="utf-8")
    except OSError:
        text = "Nothing sent yet. After your next game, the exact summary sent appears here."
    dlg = QDialog(parent)
    dlg.setWindowTitle("Owlingstone – what is sent")
    lay = QVBoxLayout(dlg)
    view = QPlainTextEdit(text)
    view.setReadOnly(True)
    lay.addWidget(view)
    dlg.resize(460, 520)
    dlg.exec()


def delete_shared(tray):
    if QMessageBox.question(None, "Owlingstone", "Delete every game and feedback you shared from the server, "
                            "and stop sharing?") != QMessageBox.Yes:
        return False
    def run():
        try:
            ok = share.delete_remote()
        except OSError:
            ok = False
        if tray:
            tray.showMessage("Owlingstone", "Your shared data was deleted." if ok else
                             "Could not reach the server. Try again later.", QSystemTrayIcon.Information, 6000)
    threading.Thread(target=run, name="share-delete", daemon=True).start()
    return True


def tray_icon(app, win):
    """Icône près de l'horloge (comme HDT) : la fenêtre n'a ni cadre ni bouton dans la barre des tâches,
    c'est donc d'ici qu'on la montre, ouvre ses stats, met à jour ou quitte l'application."""
    tray = QSystemTrayIcon(app.windowIcon(), app)
    tray.setToolTip(f"Owlingstone v{__version__}")
    menu = QMenu()
    show = QAction("Show window (turns off auto-hide)", menu)
    show.triggered.connect(lambda: win.hide_box.setChecked(False))
    stats = QAction("My stats", menu)
    stats.triggered.connect(win.show_stats)
    news = QAction("What's new", menu)
    news.triggered.connect(lambda: show_whats_new(tray))
    folder = QAction("Open my data folder", menu)
    folder.triggered.connect(lambda: os.startfile(USER_DIR) if hasattr(os, "startfile") else None)
    # Toujours présent : reconstruit l'exe depuis le projet (le texte change quand une version est prête).
    tray.update_action = QAction("Rebuild / update now", menu)
    tray.update_action.triggered.connect(win.run_update)
    tray.update_action.setEnabled(update_script() is not None)
    odds = QAction("Win chance at the top", menu, checkable=True)
    odds.setChecked(win.settings.get("top_odds", True))
    odds.toggled.connect(lambda on: (win.settings.__setitem__("top_odds", on), save_settings(win.settings)))
    narrower = QAction("Side panels narrower", menu)
    narrower.triggered.connect(lambda: win.resize_panels(-PANEL_STEP))
    wider = QAction("Side panels wider", menu)
    wider.triggered.connect(lambda: win.resize_panels(PANEL_STEP))
    sharing = QAction("Share my games (anonymous)", menu, checkable=True)
    sharing.setChecked(share.enabled())
    sharing.setEnabled(bool(share.API_URL))
    sharing.toggled.connect(share.set_enabled)
    shown = QAction("Show what is sent", menu)
    shown.triggered.connect(lambda: show_shared())
    erase = QAction("Delete my shared games", menu)
    erase.triggered.connect(lambda: delete_shared(tray) and sharing.setChecked(False))
    feedback = QAction("Send feedback…", menu)
    feedback.triggered.connect(lambda: FeedbackDialog().exec())
    feedback.setEnabled(bool(share.API_URL))
    tray.sharing_action = sharing
    quit_ = QAction("Quit Owlingstone", menu)
    quit_.triggered.connect(app.quit)
    for action in (show, stats, odds, narrower, wider, news, folder, tray.update_action):
        menu.addAction(action)
    menu.addSeparator()
    for action in (feedback, sharing, shown, erase):
        menu.addAction(action)
    menu.addSeparator()
    menu.addAction(quit_)
    tray.setContextMenu(menu)
    tray.menu = menu                       # garde le menu en vie (sinon Python le libère)
    tray.show()
    return tray


def show_whats_new(tray, version=__version__):
    lines = whats_new(version)
    text = "\n".join("• " + line for line in lines[:4]) or "No notes for this version."
    tray.showMessage(f"Owlingstone v{version}", text, QSystemTrayIcon.Information, 15000)


def announce_version(win, tray):
    """Premier lancement d'une nouvelle version : on montre ce qui a changé (une seule fois)."""
    if win.settings.get("last_version") != __version__:
        if tray and win.settings.get("last_version"):
            show_whats_new(tray)
        win.settings["last_version"] = __version__
        save_settings(win.settings)


def grab_both(win, path):
    """Capture (tests) : la fenêtre principale et le panneau Lobby côte à côte dans un seul PNG."""
    left = win.grab()
    if not win.hud.isVisible():
        left.save(path)
        return
    right = win.hud.grab()
    out = QPixmap(left.width() + 10 + right.width(), max(left.height(), right.height()))
    out.fill(Qt.black)
    p = QPainter(out)
    p.drawPixmap(0, 0, left)
    p.drawPixmap(left.width() + 10, 0, right)
    p.end()
    out.save(path)


def run(replay_path=None, stop_turn=None, screenshot=None):
    app = QApplication.instance() or QApplication(sys.argv)
    app.setWindowIcon(QIcon(str(ICON_PATH)))
    # La fenêtre se cache quand le jeu n'est pas devant : on ne quitte que par l'icône de notification.
    app.setQuitOnLastWindowClosed(bool(replay_path))
    lock = None
    if not replay_path:
        # Une seule fenêtre à la fois, même si on relance l'exe.
        lock = QLockFile(QDir.tempPath() + "/owlingstone.lock")
        if not lock.tryLock(100):
            return 0
    win = MainWindow(replay_mode=bool(replay_path))
    tray = tray_icon(app, win) if not replay_path and QSystemTrayIcon.isSystemTrayAvailable() else None
    win.tray = tray
    if not replay_path:
        announce_version(win, tray)
        ask_sharing()
        if tray:
            tray.sharing_action.setChecked(share.enabled())
        send_in_background()                   # avis ou parties restés en file
        threading.Thread(target=share.download_stats, name="stats", daemon=True).start()
    worker = LogWorker(replay_path, stop_turn)
    worker.snapshot.connect(win.on_snapshot)   # le signal du fil de lecture met à jour la fenêtre
    if replay_path or not win.hide_box.isChecked():
        win.show()                             # sinon : check_focus la montre quand le jeu est devant
        win.place_hud()
    if screenshot:
        # Mode test : capture la fenêtre en PNG dès que l'instantané est affiché, puis quitte.
        def shoot(_snap):
            QTimer.singleShot(300, lambda: (win.place_hud(), grab_both(win, screenshot), app.quit()))
        worker.snapshot.connect(shoot)
    worker.start()
    code = app.exec()
    worker.requestInterruption()
    worker.wait(2000)
    if tray:
        tray.hide()
    return code
