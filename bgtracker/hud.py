"""Panneau de droite (à droite du jeu), comme le HUD Battlegrounds de HDT, avec 2 onglets :

Onglet "Comps" (comps.py, rules.py, advice.py) :
- En haut : ce que je joue (comp reconnue, cartes qui manquent), les achats conseillés, ou au choix
  du héros les héros proposés (même texte que celui calculé par la fenêtre principale).
- Puis toutes les comps jouables dans ce lobby, mes préférées d'abord (rules.yaml) : placement moyen,
  difficulté, nombre de cartes possédées. Un clic ouvre la fiche : cartes core / add-ons (✓ possédée,
  or = dans la boutique), quand s'engager, comment jouer, notes de mes règles. La comp ouverte est
  marquée ★ dans la liste des tiers. Les comps cachées par mes règles sont listées en bas.

Onglet "Lobby" :
- En haut : mon prochain adversaire (ou le joueur cliqué) avec le plateau qu'il avait la dernière fois
  que je l'ai affronté, son tier, ses PV, ses triples, et la comp que ce plateau ressemble le plus.
- Au milieu : les 8 joueurs par place. ▶ = mon prochain adversaire, en vert = moi, grisé = éliminé.
  Un clic sur un joueur montre son dernier plateau ; un 2e clic revient au prochain adversaire.
- En bas : mes combats de la partie (W gagné, L perdu, T égalité) et mes parties du jour.
Tout vient de l'instantané (snapshot) construit dans ui.py : le panneau ne lit jamais le log lui-même.
"""
import html
from datetime import date

from PySide6.QtCore import QRectF, Qt, Signal
from PySide6.QtGui import QColor, QPainter
from PySide6.QtWidgets import (QFrame, QHBoxLayout, QLabel, QPushButton, QScrollArea, QStackedWidget,
                               QVBoxLayout, QWidget)

from .advice import PLAYING_MIN_CONFIDENCE
from .cards import get_db
from .comps import available_comps, cards_from_ids, lobby_comps, match_comps
from .history import HISTORY_PATH, load_history
from .rules import judge
from .widgets import (HEALTH, Thumbs, Tile, card_tile, pill_html, place_color, plain_text,
                      tier_color)

HUD_WIDTH = 300
RESULT_COLOR = {"W": "#3f9d55", "L": "#c43c3c", "T": "#5c606b"}
RESULT_WORD = {"W": "you won", "L": "you lost", "T": "tie"}


def clear(layout):
    """Vide une mise en page (les anciens widgets sont détruits)."""
    while layout.count():
        item = layout.takeAt(0)
        if item.widget():
            item.widget().deleteLater()


def short_hero(name, width=18):
    return name if len(name) <= width else name[:width - 1] + "…"


def ordinal(n):
    return f"{n}{'st' if n == 1 else 'nd' if n == 2 else 'rd' if n == 3 else 'th'}"


def today_games(records, day=None):
    """Mes parties du jour (les plus anciennes d'abord)."""
    day = day or date.today().isoformat()
    return sorted((r for r in records if r.get("date") == day), key=lambda r: r.get("time", ""))


class ClickTile(Tile):
    """Tuile cliquable : émet sa clé (id du joueur, id de la comp...)."""
    clicked = Signal(object)

    def __init__(self, key, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.key = key
        self.setCursor(Qt.PointingHandCursor)

    def mousePressEvent(self, event):
        self.clicked.emit(self.key)


def player_tile(player, art, selected=False, height=None):
    """Joueur du lobby façon HDT : portrait du héros, place dans la pastille, PV / tier / triples."""
    badges = []
    if player["health"] is not None and not player["dead"]:
        badges.append((player["health"], HEALTH))
    if player["tier"]:
        badges.append((f"T{player['tier']}", tier_color(player["tier"])))
    if player["triples"]:
        badges.append((f"{player['triples']}★", "#8a6d1e"))
    fight = player.get("fight")
    if fight:
        badges.append((fight["result"], RESULT_COLOR[fight["result"]]))
    state = "selected" if selected else "me" if player["me"] else "next" if player["next"] else ""
    tip = [player["hero"]]
    if fight:
        tip.append(f"Last fight, turn {fight['turn']}: {RESULT_WORD[fight['result']]}"
                   + (f" ({fight['damage']} damage)" if fight["damage"] else ""))
    return ClickTile(player["id"], player["hero"], art, gem=player["place"] or "?",
                     gem_color="#b8962a" if player["next"] else "#3a3e4a", badges=badges, state=state,
                     mark="▶" if player["next"] else "", dim=player["dead"], tooltip="\n".join(tip), height=height,
                     card_id=player.get("hero_id"))


# Sections repliées (clé = début du titre, ex. "buy", "turn plan"), retenues dans settings.json.
FOLDED = set()
_on_fold = []                                  # fonctions appelées quand on replie / déplie


class SectionHeader(QLabel):
    """Titre de section cliquable : un clic replie ou déplie tout ce qui suit jusqu'au titre suivant."""

    def __init__(self, text, key=None):
        super().__init__()
        self.setObjectName("section")
        self.setCursor(Qt.PointingHandCursor)
        self.key = key or text.split("·")[0].strip().lower()
        self.setToolTip("Click to fold / unfold")
        self.set_title(text)

    def set_title(self, text):
        self._text = text.upper()
        self.setText(("▸ " if self.folded else "▾ ") + self._text)

    @property
    def folded(self):
        return self.key in FOLDED

    def mousePressEvent(self, event):
        FOLDED.symmetric_difference_update({self.key})
        self.set_title(self._text)
        for fn in _on_fold:
            fn()


def label_section(text):
    """Titre simple, non repliable (dans une carte de comp)."""
    label = QLabel(text.upper())
    label.setObjectName("section")
    return label


def section(text, key=None):
    return SectionHeader(text, key)


def apply_folds(layout, hidden=False):
    """Cache ce qui suit un titre replié (jusqu'au titre suivant), et remontre ce qu'on avait caché
    nous-mêmes quand on déplie. On ne touche pas aux widgets cachés pour une autre raison."""
    for i in range(layout.count()):
        item = layout.itemAt(i)
        w, sub = item.widget(), item.layout()
        if isinstance(w, SectionHeader):
            w.set_title(w._text)
            hidden = w.folded
        elif w is not None:
            if hidden and not w.isHidden():
                w.setProperty("folded", True)
                w.hide()
            elif not hidden and w.property("folded"):
                w.setProperty("folded", False)
                w.show()
        elif sub is not None:
            apply_folds(sub, hidden) if hidden else apply_folds(sub)


def card_list(cards, owned, shop):
    """Noms de cartes : ✓ vert si je l'ai, or si elle est dans la boutique, (Tn) = son tier."""
    parts = []
    for c in cards:
        name = f"{html.escape(c.name)} <span style='color:#9aa0ad'>T{c.tier}</span>" if c.tier else html.escape(c.name)
        if c.id in owned:
            parts.append(f"<span style='color:#7fc97f'>✓ {name}</span>")
        elif c.id in shop:
            parts.append(f"<span style='color:#c9a227'>● {name}</span>")
        else:
            parts.append(name)
    return ", ".join(parts) or "–"


class CompCard(QFrame):
    """Une comp façon guide de HDT : nom, badges (placement moyen, difficulté, mes règles), aperçu des
    cartes clés en images ; ouverte : toutes ses cartes en tuiles, quand s'engager, comment jouer."""
    clicked = Signal(str)

    def __init__(self, comp, verdict, owned, shop, playing, is_open, tiles, art_on):
        super().__init__()
        self.setObjectName("comp")
        self.comp_id = comp.id
        self.setCursor(Qt.PointingHandCursor)
        self.setProperty("state", "playing" if playing else "open" if is_open else "")
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 5, 6, 6)
        lay.setSpacing(3)
        cards = comp.core + comp.addons
        have = sum(c.id in owned for c in cards)
        avg = comp.stats.get("avg")
        pills = []
        if playing:
            pills.append(pill_html("PLAYING", "#b8962a", "#111"))
        if verdict.bonus > 0:
            pills.append(pill_html("👍 my rule", "#2f6b3c"))
        elif verdict.bonus < 0:
            pills.append(pill_html("👎 my rule", "#7a2e2e"))
        if avg:
            pills.append(pill_html(f"{avg:.2f}", place_color(avg)))
        if comp.difficulty:
            pills.append(pill_html(comp.difficulty, "#3a3e4a"))
        if have:
            pills.append(pill_html(f"{have}/{len(cards)} ✓", "#2f6b3c"))
        name = QLabel(f"<span style='font-size:13px;font-weight:bold;color:{'#f0d27a' if playing else '#ffffff'}'>"
                      f"{html.escape(comp.name)}</span>")
        name.setTextFormat(Qt.RichText)
        name.setWordWrap(True)
        lay.addWidget(name)
        badges = QLabel(" ".join(pills))
        badges.setTextFormat(Qt.RichText)
        lay.addWidget(badges)
        if not is_open:
            lay.addWidget(Thumbs(comp.core + comp.addons, tiles, owned, shop, art_on))
        else:
            lay.addWidget(label_section("Core cards"))
            for c in comp.core:
                lay.addWidget(card_tile(c, tiles, art_on, state="owned" if c.id in owned else "shop" if c.id in shop else ""))
            if comp.addons:
                lay.addWidget(label_section("Add-ons"))
                for c in comp.addons:
                    lay.addWidget(card_tile(c, tiles, art_on, state="owned" if c.id in owned else "shop" if c.id in shop else ""))
            lines = []
            if comp.when_to_commit:
                lines.append(f"<b style='color:#f0d27a'>Commit when</b><br>{html.escape(comp.when_to_commit)}")
            if comp.how_to_play:
                lines.append(f"<b style='color:#f0d27a'>How to play</b><br>{html.escape(comp.how_to_play)}")
            if comp.stats.get("games"):
                lines.append(f"<span style='color:#9aa0ad'>Avg place {avg:.2f} over {comp.stats['games']:,} games "
                             f"(community data)</span>")
            for note in verdict.notes:
                lines.append(f"<b style='color:#7fe07f'>My rule:</b> {html.escape(note)}")
            if lines:
                text = QLabel("<br><br>".join(lines))
                text.setTextFormat(Qt.RichText)
                text.setWordWrap(True)
                lay.addWidget(text)
        self.setToolTip("Click to open / close. The open comp's cards get ★ in the tier list.")

    def mousePressEvent(self, event):
        self.clicked.emit(self.comp_id)


def page_scroll(page):
    """Un onglet entier dans une zone qui défile verticalement : quand le plan du tour, les achats et
    les comps ne tiennent pas, l'onglet défile au lieu d'agrandir la fenêtre (qui débordait alors de
    l'écran). Jamais de défilement horizontal : le texte revient à la ligne."""
    area = QScrollArea()
    area.setWidgetResizable(True)
    area.setFrameShape(QFrame.NoFrame)
    area.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
    area.setWidget(page)
    return area


class LobbyWindow(QWidget):
    """Fenêtre sans cadre posée à droite du jeu. MainWindow la place, la montre et la cache."""

    def __init__(self, main):
        super().__init__()
        self.main = main                      # pour les comps, les images de cartes et les réglages
        self.snap = {"in_game": False}
        self.selected = None                  # PLAYER_ID cliqué (None = mon prochain adversaire)
        self._history_cache = (None, [])      # (date de modification du fichier, parties)
        self._last_key = None
        self.setWindowTitle("Owlingstone – Lobby")
        self.setWindowFlags(Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.Tool)
        self.setWindowOpacity(0.93)
        self.setStyleSheet(main.styleSheet())
        self.resize(HUD_WIDTH, 620)

        self.open_comp = None                 # comp ouverte : None = celle que je joue, "" = aucune
        self._comps_key = None

        outer = QVBoxLayout(self)
        outer.setContentsMargins(8, 8, 8, 8)
        outer.setSpacing(4)
        # Compteurs (comme HDT) : bonus de mes sorts de taverne, valeur de mes Gemmes de sang.
        self.counters = QLabel()
        self.counters.setTextFormat(Qt.RichText)
        self.counters.setWordWrap(True)
        outer.addWidget(self.counters)
        tabs = QHBoxLayout()
        self.tab_buttons = {}
        for name in ("Comps", "Lobby"):
            b = QPushButton(name)
            b.setCheckable(True)
            b.clicked.connect(lambda _=False, n=name: self.show_tab(n))
            tabs.addWidget(b)
            self.tab_buttons[name] = b
        outer.addLayout(tabs)
        self.stack = QStackedWidget()
        outer.addWidget(self.stack, 1)

        # --- onglet Comps
        comps_page = QWidget()
        cl = QVBoxLayout(comps_page)
        self.page_layouts = [cl]
        cl.setContentsMargins(0, 0, 0, 0)
        cl.setSpacing(4)
        self.advice_box = QVBoxLayout()       # ce que je joue / achats / choix du héros (en images)
        self.advice_box.setSpacing(2)
        cl.addLayout(self.advice_box)
        self.comps_title = SectionHeader("Comps", key="comps")
        cl.addWidget(self.comps_title)
        self.comps_scroll = QScrollArea()
        self.comps_scroll.setWidgetResizable(True)
        self.comps_scroll.setMinimumHeight(160)
        cl.addWidget(self.comps_scroll, 1)
        cl.addStretch(0)                      # prend la place libre quand la liste des comps est repliée
        self.stack.addWidget(page_scroll(comps_page))

        # --- onglet Lobby
        lobby_page = QWidget()
        root = QVBoxLayout(lobby_page)
        self.page_layouts.append(root)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(4)
        self.stack.addWidget(page_scroll(lobby_page))
        self.combat_box = QVBoxLayout()       # chances du combat (comme BobsBuddy), voir render_combat
        self.combat_box.setSpacing(2)
        root.addLayout(self.combat_box)
        self._combat_key = None
        root.addWidget(section("Opponent"))
        self.title = QLabel()
        self.title.setTextFormat(Qt.RichText)
        self.title.setWordWrap(True)
        root.addWidget(self.title)
        self.hero_box = QVBoxLayout()         # portrait de l'adversaire montré
        root.addLayout(self.hero_box)
        self.info = QLabel()
        self.info.setTextFormat(Qt.RichText)
        self.info.setWordWrap(True)
        root.addWidget(self.info)
        self.board = QVBoxLayout()            # plateau de l'adversaire montré
        self.board.setSpacing(2)
        root.addLayout(self.board)
        root.addWidget(section("Lobby"))
        self.scroll = QScrollArea()
        self.scroll.setWidgetResizable(True)
        self.scroll.setMinimumHeight(160)
        root.addWidget(self.scroll, 1)
        root.addStretch(0)
        # --- sous les 2 onglets : mes combats et mes parties du jour
        self.fights = QLabel()
        self.fights.setTextFormat(Qt.RichText)
        self.fights.setWordWrap(True)
        outer.addWidget(self.fights)
        self.session = QLabel()
        self.session.setTextFormat(Qt.RichText)
        self.session.setWordWrap(True)
        self.session.setObjectName("status")
        outer.addWidget(self.session)
        FOLDED.update(main.settings.get("folded", []))
        _on_fold.append(self.on_fold)
        self.show_tab(main.settings.get("hud_tab", "Comps"), save=False)
        self.render()

    def show_tab(self, name, save=True):
        if name not in self.tab_buttons:
            name = "Comps"
        for n, b in self.tab_buttons.items():
            b.setChecked(n == name)
        self.stack.setCurrentIndex(0 if name == "Comps" else 1)
        if save:
            self.main.settings["hud_tab"] = name
            self.main.save_settings()

    # -------------------------------------------------------------- données
    def on_snapshot(self, snap):
        if not snap.get("in_game") or (snap.get("turn") or 0) < (self.snap.get("turn") or 0):
            self.selected = None              # nouvelle partie
            self.open_comp = None
        self.snap = snap
        self.render()

    def on_comp_clicked(self, comp_id):
        shown = self.shown_comp()
        self.open_comp = "" if comp_id == shown else comp_id
        self.main.redraw_cards()              # les ★ de la liste des tiers suivent la comp ouverte
        self.render(force=True)

    def shown_comp(self):
        """Id de la comp ouverte (celle cliquée, sinon celle que je joue), ou None."""
        return self.main.playing_comp if self.open_comp is None else (self.open_comp or None)

    def on_player_clicked(self, pid):
        self.selected = None if pid == self.selected else pid
        self.render(force=True)

    def history(self):
        """Mes parties (relues seulement si le fichier a changé)."""
        try:
            mtime = HISTORY_PATH.stat().st_mtime
        except OSError:
            return []
        if self._history_cache[0] != mtime:
            try:
                self._history_cache = (mtime, load_history())
            except (OSError, ValueError):
                self._history_cache = (mtime, [])
        return self._history_cache[1]

    def shown_player(self, players):
        """Le joueur dont on montre le plateau : celui cliqué, sinon mon prochain adversaire,
        sinon (pendant le combat, quand le jeu a déjà choisi le suivant) le dernier affronté."""
        by_id = {p["id"]: p for p in players}
        if self.selected in by_id:
            return by_id[self.selected], False
        nxt = next((p for p in players if p["next"]), None)
        if nxt:
            return nxt, True
        fights = self.snap.get("fights") or []
        if fights and fights[-1]["opponent"] in by_id:
            return by_id[fights[-1]["opponent"]], False
        return None, False

    def on_fold(self):
        self.main.settings["folded"] = sorted(FOLDED)
        self.main.save_settings()
        self.apply_folds()

    def apply_folds(self):
        for layout in self.page_layouts:
            apply_folds(layout)

    # -------------------------------------------------------------- affichage
    def render(self, force=False):
        self._render(force)
        self.apply_folds()

    def _render(self, force=False):
        self.render_counters()
        self.render_comps(force)
        s = self.snap
        if force or repr(s.get("combat")) != self._combat_key:
            self._combat_key = repr(s.get("combat"))
            render_combat(self.combat_box, s.get("combat") if s.get("in_game") else None)
        players = s.get("players") or []
        key = (repr(players), repr(s.get("fights")), self.selected, s.get("in_game"),
               self.main.art_box.isChecked(), self._history_cache[0])
        if key == self._last_key and not force:
            return
        self._last_key = key
        self.render_opponent(players)
        rows = QWidget()
        lay = QVBoxLayout(rows)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(2)
        art_on = self.main.art_box.isChecked()
        for p in players:
            row = player_tile(p, self.main.tiles.get(p["hero_id"]) if art_on and p.get("hero_id") else None,
                              p["id"] == self.selected)
            row.clicked.connect(self.on_player_clicked)
            lay.addWidget(row)
        if not players:
            waiting = QLabel("Players appear when the game starts.")
            waiting.setObjectName("status")
            lay.addWidget(waiting)
        lay.addStretch(1)
        self.scroll.setWidget(rows)
        self.render_fights()
        self.render_session()

    def render_comps(self, force=False):
        main, s = self.main, self.snap
        races = s.get("races") if s.get("races_complete") else None
        owned = set(s.get("board", [])) | set(s.get("hand", []))
        shop = set(s.get("shop", []))
        ctx = {"races": races, "deity": s.get("deity"), "hero": s.get("hero"),
               "hero_power": [name for name, _ in s.get("hero_powers") or []] or None}
        key = (id(main.comps), tuple(races or ()), tuple(sorted(owned)), tuple(sorted(shop)), ctx["deity"],
               ctx["hero"], repr(ctx["hero_power"]), self.shown_comp(), main.playing_comp)
        plan = main.advice.get("plan") or {}
        key += (repr(main.advice.get("heroes")), tuple(p.card.id for p in main.advice.get("buy") or ()),
                repr({k: v for k, v in plan.items() if k not in ("buy", "fight")}),
                main.comps_error, self.main.art_box.isChecked(), _trinket_key(main.advice.get("trinkets")))
        if key == self._comps_key and not force:
            return
        self._comps_key = key
        self.render_advice(owned, shop)
        shown = lobby_comps(main.comps, races, main.rules, **{k: ctx[k] for k in ("deity", "hero", "hero_power")})
        shown.sort(key=lambda c: c.id != main.playing_comp)   # celle que je joue en premier
        hidden = [c for c in available_comps(main.comps, races) if judge(c.id, main.rules, ctx).hidden]
        where = "in this lobby" if races else "(races not known yet: all)"
        self.comps_title.set_title(f"COMPS {where.upper()}  ({len(shown)})")
        w = QWidget()
        lay = QVBoxLayout(w)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(4)
        open_id = self.shown_comp()
        for comp in shown:
            row = CompCard(comp, judge(comp.id, main.rules, ctx), owned, shop, comp.id == main.playing_comp,
                           comp.id == open_id, main.tiles, main.art_box.isChecked())
            row.clicked.connect(self.on_comp_clicked)
            lay.addWidget(row)
        if hidden:
            note = QLabel("Hidden by my rules: " + ", ".join(html.escape(c.name) for c in hidden))
            note.setObjectName("status")
            note.setWordWrap(True)
            lay.addWidget(note)
        lay.addStretch(1)
        self.comps_scroll.setWidget(w)

    def render_counters(self):
        s = self.snap
        bonus = s.get("bonuses") or {}
        chips = []
        spell = bonus.get("spell") or (0, 0)
        if s.get("in_game"):
            chips.append(pill_html(f"Turn {s.get('turn') or 0}", "#b8962a", "#111"))
            if s.get("tier"):
                chips.append(pill_html(f"Tier {s['tier']}", tier_color(s["tier"])))
            chips.append(pill_html(f"✨ Tavern spells +{spell[0]}/+{spell[1]}", "#2d4f7a" if any(spell) else "#3a3e4a"))
            gem = bonus.get("gem") or (1, 1)
            if "QUILBOAR" in (s.get("races") or []) or tuple(gem) != (1, 1):
                chips.append(pill_html(f"🩸 Blood Gem +{gem[0]}/+{gem[1]}", "#7a2e3a"))
        self.counters.setText(" ".join(chips))
        self.counters.setVisible(bool(chips))
        self.counters.setToolTip("Tavern spells: extra attack/health my Tavern spells give.\n"
                                 "Blood Gem: what one Blood Gem gives right now (+1/+1 at the start).")

    def render_advice(self, owned, shop):
        """Haut de l'onglet Comps : héros à choisir, comp jouée et cartes qui manquent, achats conseillés."""
        clear(self.advice_box)
        main, adv = self.main, self.main.advice or {}
        box, art_on = self.advice_box, main.art_box.isChecked()
        if main.comps_error:
            err = QLabel(f"<span style='color:#e57373'>{html.escape(main.comps_error)}</span>")
            err.setWordWrap(True)
            box.addWidget(err)
            return
        if not self.snap.get("in_game"):
            box.addWidget(QLabel("<span style='color:#9aa0ad'>Waiting for a Battlegrounds game…</span>"))
            return
        if adv.get("deity"):
            box.addWidget(QLabel(f"<span style='color:#9aa0ad'>Deity:</span> {pill_html(html.escape(adv['deity']), '#5b3a7a')}"))
        powers = self.snap.get("hero_powers") or []
        if powers and not adv.get("heroes"):
            box.addWidget(section("My hero power" + ("s" if len(powers) > 1 else "")))
            db = get_db()
            for name, card_id in powers:
                card = db.get(card_id)
                box.addWidget(Tile(name, main.tiles.get(card_id) if art_on else None,
                                   gem=card.cost if card and card.cost is not None else "?", gem_color="#2d4f7a",
                                   card_id=card_id, tooltip=plain_text(card.text) if card else ""))
        if adv.get("heroes"):
            box.addWidget(section("Pick a hero (avg place)"))
            hero_comps = adv.get("hero_comps") or {}
            for i, (name, hero_id, st) in enumerate(adv["heroes"]):
                badges = [(f"{st['avg']:.2f}", place_color(st["avg"]))] if st else [("?", "#5c606b")]
                box.addWidget(Tile(name, main.tiles.get(hero_id) if art_on and hero_id else None, gem=i + 1,
                                   gem_color="#b8962a" if i == 0 and st else "#3a3e4a", badges=badges,
                                   state="playing" if i == 0 and st else "", height=34, card_id=hero_id))
                best = hero_comps.get(name)
                parts = []
                if best:
                    # Comps où ce héros est parmi les meilleurs (placement moyen du héros avec la comp)
                    parts.append("<span style='color:#9aa0ad'>best in:</span> " + " · ".join(
                        f"{html.escape(c.name)} <b style='color:{place_color(avg)}'>{avg:.2f}</b>" for c, avg, _ in best))
                curve_note = (adv.get("hero_notes") or {}).get(name)
                if curve_note:
                    parts.append(f"<span style='color:#9aa0ad'>{html.escape(curve_note)}</span>")
                if parts:
                    note = QLabel("<br>".join(parts))
                    note.setWordWrap(True)
                    box.addWidget(note)
            return
        if adv.get("trinkets"):
            render_trinkets(box, adv["trinkets"], main.tiles, art_on)
        if adv.get("plan"):
            render_plan(box, adv["plan"])
        best, alt = adv.get("playing"), adv.get("alt")
        if best:
            box.addWidget(section(f"Playing · {best.comp.name} · {best.confidence}%"))
            for c in best.missing_core[:3]:
                box.addWidget(card_tile(c, main.tiles, art_on, state="shop" if c.id in shop else "", mark="Need"))
            if not best.missing_core:
                box.addWidget(QLabel("<span style='color:#7fe07f'>All core cards ✓</span>"))
            if alt:
                box.addWidget(QLabel(f"<span style='color:#9aa0ad'>or {html.escape(alt.comp.name)} ({alt.confidence}%)</span>"))
        if adv.get("buy"):
            box.addWidget(section("Buy"))
            planned = {c.id for c in (adv.get("plan") or {}).get("plan_buys", ())}
            for pick in adv["buy"]:
                # Avec le plan du tour : ce que mon or permet d'acheter en vif, l'alternative en grisé
                spare = bool(adv.get("plan")) and pick.card.id not in planned
                tile = card_tile(pick.card, main.tiles, art_on, state="" if spare else "buy",
                                 mark="" if spare else "▶", dim=spare)
                tile.tooltip = ", ".join(pick.reasons) + "\n" + plain_text(pick.card.text)
                box.addWidget(tile)

    def render_opponent(self, players):
        clear(self.board)
        clear(self.hero_box)
        if not self.snap.get("in_game"):
            self.title.setText("<b>Lobby</b> <span style='color:#9aa0ad'>· waiting for a game…</span>")
            self.info.setText("")
            return
        player, is_next = self.shown_player(players)
        if player is None:
            self.title.setText("<b>Next opponent</b> <span style='color:#9aa0ad'>· not known yet</span>")
            self.info.setText("")
            return
        label = "Next opponent" if is_next else "You" if player["me"] else "Player"
        self.title.setText(f"<b style='color:#c9a227'>{label.upper()}</b>")
        art = self.main.tiles.get(player["hero_id"]) if self.main.art_box.isChecked() and player.get("hero_id") else None
        hero = player_tile(player, art, height=40)
        hero.clicked.connect(self.on_player_clicked)
        self.hero_box.addWidget(hero)
        fight = player.get("fight")
        if player["me"]:
            self.info.setText("<span style='color:#9aa0ad'>That's you. Click another player to see their board.</span>")
            return
        if not fight:
            self.info.setText("<span style='color:#9aa0ad'>Not fought yet: board unknown.</span>")
            return
        result = (f"<b style='color:{RESULT_COLOR[fight['result']]}'>{RESULT_WORD[fight['result']]}</b>"
                  + (f" ({fight['damage']} dmg)" if fight["damage"] else ""))
        cards = cards_from_ids([m["card_id"] for m in fight["board"]])
        comp = self.guess_comp(cards)
        self.info.setText(f"<span style='color:#9aa0ad'>Their board on turn {fight['turn']}:</span> {result}"
                          + (f"<br><span style='color:#9aa0ad'>Plays:</span> {comp}" if comp else ""))
        db = get_db()
        for m in fight["board"]:
            card = db.base(m["card_id"])
            if card is None:
                continue
            self.board.addWidget(card_tile(card, self.main.tiles, self.main.art_box.isChecked(),
                                           attack=m["attack"], health=m["health"], golden=m["golden"]))
        if not fight["board"]:
            empty = QLabel("(empty board)")
            empty.setObjectName("status")
            self.board.addWidget(empty)

    def guess_comp(self, cards):
        """La comp de mes données qui ressemble le plus à ce plateau (sans mes règles : c'est le sien)."""
        if not cards or not self.main.comps:
            return ""
        races = self.snap.get("races") if self.snap.get("races_complete") else None
        found = match_comps(self.main.comps, cards, (), races, top=1, min_confidence=PLAYING_MIN_CONFIDENCE)
        if not found:
            return ""
        return f"<b>{html.escape(found[0].comp.name)}</b> <span style='color:#9aa0ad'>({found[0].confidence}%)</span>"

    def render_fights(self):
        fights = self.snap.get("fights") or []
        if not self.snap.get("in_game") or not fights:
            self.fights.setText("")
            return
        marks = " ".join(pill_html(f["result"], RESULT_COLOR[f["result"]]) for f in fights[-12:])
        won = sum(f["result"] == "W" for f in fights)
        lost = sum(f["result"] == "L" for f in fights)
        self.fights.setText(f"<span style='color:#9aa0ad'>My fights ({won}-{lost}):</span> {marks}")

    def render_session(self):
        games = today_games(self.history())
        if not games:
            self.session.setText("Today: no finished game yet.")
            return
        places = [g["placement"] for g in games]
        avg = sum(places) / len(places)
        top4 = sum(p <= 4 for p in places)
        listed = ", ".join(ordinal(p) for p in places[-8:])
        self.session.setText(f"Today: {len(games)} game{'s' if len(games) > 1 else ''} · avg {avg:.1f} · "
                             f"top 4: {top4}/{len(games)}<br>{listed}")


# ------------------------------------------------------------------- chances du combat (combat.py)
WIN, TIE, LOSS = "#3f9d55", "#5c606b", "#c43c3c"


class OddsBar(QWidget):
    """Barre victoire / égalité / défaite, comme celle de BobsBuddy, avec les 3 pourcentages toujours écrits
    dessous (Win à gauche, Tie au milieu, Loss à droite) : un petit morceau de barre n'a pas la place pour
    son chiffre."""
    LABEL_H = 15

    def __init__(self, win, tie, loss, height=22):
        super().__init__()
        self.parts = [(win, WIN), (tie, TIE), (loss, LOSS)]
        self.bar_h = height
        self.setFixedHeight(height + self.LABEL_H)

    def paintEvent(self, event):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        w, h = self.width(), self.bar_h
        x = 0.0
        for i, (pct, color) in enumerate(self.parts):
            width = w * pct / 100 if i < 2 else w - x
            if width <= 0:
                continue
            p.fillRect(QRectF(x, 0, width, h), QColor(color))
            x += width
        font = p.font()
        font.setBold(True)
        p.setFont(font)
        row = QRectF(0, h, w, self.LABEL_H)
        texts = ("Win", "Tie", "Loss")
        aligns = (Qt.AlignLeft, Qt.AlignHCenter, Qt.AlignRight)
        colors = ("#7fe07f", "#c8ccd4", "#ff7a7a")
        for (pct, _), word, align, color in zip(self.parts, texts, aligns, colors):
            p.setPen(QColor(color))
            p.drawText(row, align | Qt.AlignVCenter, f"{word} {pct:.0f}%")


def _trinket_key(t):
    if not t:
        return None
    return (tuple((p.card.id, p.cost, p.score, tuple(p.reasons)) for p in t.get("offer") or ()),
            tuple((t.get("hint") or {}).get("lines", ())))


def render_trinkets(box, data, tiles, art_on):
    """Trinkets (trinkets.py) : l'offre classée (le meilleur en or, raisons dessous), ou l'aide avant l'offre."""
    picks = data.get("offer") or []
    if picks:
        box.addWidget(section("Trinket · pick (expected place)", key="trinket-offer"))
        for i, p in enumerate(picks):
            best = i == 0 and p.affordable
            tile = Tile(p.card.name, tiles.get(p.card.id) if art_on else None, gem=p.cost, gem_color="#b8962a",
                        badges=[(f"{p.score:.2f}", place_color(p.score))], state="playing" if best else "",
                        mark="▶" if best else "", dim=not p.affordable, card_id=p.card.id,
                        tooltip=plain_text(p.card.text))
            box.addWidget(tile)
            note = QLabel(f"<span style='color:#9aa0ad'>{html.escape(' · '.join(p.reasons))}</span>")
            note.setWordWrap(True)
            box.addWidget(note)
        return
    hint = data.get("hint")
    if hint:
        box.addWidget(section("Next Trinket offer", key="trinket-hint"))
        lines = [html.escape(l) for l in hint["lines"]]
        lines[0] = f"<span style='color:#9aa0ad'>{lines[0]}</span>"
        label = QLabel("<br>".join(lines))
        label.setWordWrap(True)
        label.setToolTip("Trinket offers come at the start of turn 6 (Lesser) and turn 9 (Greater). Tribe Trinkets "
                         "need 2 minions of the tribe on turn 6, 3 on turn 9 (board and hand when the turn starts). "
                         "One Trinket of your main tribe is always offered; one tribe you are not in can show up "
                         "2 gold cheaper (pivot). Numbers = average place of players who took it (bgbuddy.gg).")
        box.addWidget(label)


def render_plan(box, plan):
    """Plan du tour (planner.turn_plan) : danger pour ma vie, prochain adversaire attendu, monter ou pas,
    vendre, relancer, geler. Les achats suivent dans la section "Buy"."""
    f = plan["fight"]
    s = f["base"]
    word, color, why = plan["danger"]
    box.addWidget(section(f"Turn plan · {f['gold']} gold"))
    cap = f" (cap {f['cap']})" if f.get("cap") else ""
    lines = [f"{pill_html('Danger ' + word, color, '#111')} <span style='color:#9aa0ad'>{html.escape(why)}</span>",
             f"<span style='color:#9aa0ad'>Next fight vs</span> {html.escape(f['opponent'] or '?')} "
             f"<span style='color:#9aa0ad'>T{f['opp_tier']}" + (f", {f['opp_health']} hp" if f.get("opp_health") is not None else "")
             + f"</span>: <b>{s['win']:.0f}%</b> win · <b>{s['tie']:.0f}%</b> tie · <b>{s['loss']:.0f}%</b> loss · expected "
             f"<b style='color:{LOSS}'>-{s['avg_taken']:.1f}</b>{cap}"
             + (f" · ☠ {s['lethal_me']:.0f}%" if s["lethal_me"] >= 0.5 else ""),
             f"<span style='color:#9aa0ad'>Their board: {html.escape(f['basis'])} · my board "
             f"{_strength_word(f['strength'])} for turn {f['turn']} (with hand)</span>"]
    lvl, why = plan["level"], plan["level_why"]
    if lvl is True:
        lines.append(f"<b style='color:#7fe07f'>⬆ LEVEL</b> {html.escape(why)}")
    elif lvl is False:
        lines.append(f"<b style='color:#e0a050'>✋ DON'T LEVEL</b> <span style='color:#9aa0ad'>{html.escape(why)}</span>")
    elif f.get("upgrade_cost") is not None:
        lines.append(f"<span style='color:#9aa0ad'>Level: {html.escape(why)}</span>")
    if plan.get("sell"):
        lines.append(f"<b style='color:#e0a050'>$ SELL</b> {html.escape(plan['sell'])} <span style='color:#9aa0ad'>"
                     "to make room</span>")
    if plan.get("roll"):
        pro = f" <span style='color:#9aa0ad'>· {html.escape(plan['roll_pro'])}</span>" if plan.get("roll_pro") else ""
        lines.append(f"<b style='color:#7fb0e0'>⟳ ROLL</b> {html.escape(plan['roll'])}{pro}")
    elif plan.get("roll_pro"):
        lines.append(f"<span style='color:#9aa0ad'>Roll: {html.escape(plan['roll_pro'])}</span>")
    if plan.get("freeze"):
        lines.append(f"<b style='color:#9fd8ff'>❄ FREEZE</b> {html.escape(plan['freeze'])}")
    if plan.get("power"):
        lines.append(f"<b style='color:#c9a227'>⚡ HERO POWER</b> {html.escape(plan['power'])}")
    hero_bits = []
    if plan.get("hero_note"):
        hero_bits.append(html.escape(plan["hero_note"]))
    if plan.get("hero_best"):
        hero_bits.append("best comps with your hero here: " + ", ".join(
            f"{html.escape(c.name)} {avg:.2f}" for c, avg, _ in plan["hero_best"]))
    if hero_bits:
        lines.append(f"<span style='color:#9aa0ad'>{html.escape(f['profile']['hero'] or 'Hero')}: "
                     + " · ".join(hero_bits) + "</span>")
    label = QLabel("<br>".join(lines))
    label.setWordWrap(True)
    label.setToolTip("Win chance and damage are simulated against the board your next opponent should have now: "
                     "their last board seen, grown with the turn (learned from past games), or a typical board "
                     "if you never fought them. Damage is capped like in the game. Your hand minions count as played.")
    box.addWidget(label)


def _strength_word(pct):
    if pct >= 75:
        return f"strong (top {100 - pct:.0f}%)"
    if pct <= 25:
        return f"weak (bottom {pct:.0f}%)"
    return "average"


def render_combat(layout, combat):
    """Haut de l'onglet Lobby : % de victoire / égalité / défaite du combat en cours (ou estimé contre
    le dernier plateau vu de mon prochain adversaire), dégâts moyens, risque de mourir, et ma moyenne
    sur la partie."""
    clear(layout)
    if not combat:
        return
    if combat.get("now") or combat.get("games_fights"):
        layout.addWidget(section("Win chance"))
    now = combat.get("now")
    if now:
        who = html.escape(now.get("opponent") or "opponent")
        if now["live"]:
            head = f"<b style='color:#c9a227'>COMBAT</b> <span style='color:#9aa0ad'>vs {who}</span>"
        else:
            basis = now.get("basis") or f"their turn {now['seen_turn']} board"
            head = (f"<b style='color:#c9a227'>NEXT FIGHT</b> <span style='color:#9aa0ad'>vs {who} · estimate "
                    f"vs {html.escape(basis)}</span>")
        title = QLabel(head)
        title.setWordWrap(True)
        layout.addWidget(title)
        bar = OddsBar(now["win"], now["tie"], now["loss"])
        bar.setToolTip(f"Win {now['win']:.1f}% · Tie {now['tie']:.1f}% · Loss {now['loss']:.1f}% "
                       f"({now['runs']} simulated fights)")
        layout.addWidget(bar)
        parts = []
        if now["win"]:
            parts.append(f"you deal <b>{now['dealt']:.1f}</b>")
        if now["loss"]:
            parts.append(f"you take <b>{now['taken']:.1f}</b>")
        lines = ["<span style='color:#9aa0ad'>Avg damage:</span> " + " · ".join(parts)] if parts else []
        if now["lethal_me"] >= 0.5:
            lines.append(f"<b style='color:{LOSS}'>☠ {now['lethal_me']:.0f}% chance this fight kills you</b>")
        if now["lethal"] >= 0.5:
            lines.append(f"<b style='color:{WIN}'>{now['lethal']:.0f}% chance to kill them</b>")
        if now["unknown"]:
            lines.append("<span style='color:#9aa0ad'>Not simulated: "
                         + html.escape(", ".join(now["unknown"][:4])) + "</span>")
        info = QLabel("<br>".join(lines))
        info.setWordWrap(True)
        layout.addWidget(info)
    if combat.get("games_fights"):
        n = len(combat["games_fights"])
        avg = QLabel(f"<span style='color:#9aa0ad'>This game ({n} fight{'s' if n > 1 else ''}):</span> "
                     f"avg win chance <b>{combat['avg_win']:.0f}%</b> · "
                     f"won {combat['actual_wins']} (expected {combat['expected_wins']:.1f})")
        avg.setWordWrap(True)
        avg.setToolTip("\n".join(f"Turn {g['turn']}: {g['win']:.0f}% win / {g['tie']:.0f}% tie / "
                                  f"{g['loss']:.0f}% loss → {g['result']}" for g in combat["games_fights"]))
        layout.addWidget(avg)



class CombatOverlay(QWidget):
    """Petit bandeau en haut du jeu, au-dessus du portrait de l'adversaire (comme BobsBuddy dans HDT) :
    % de victoire / égalité / défaite du combat en cours ou à venir. Les clics passent au travers."""
    WIDTH, HEIGHT = 300, 44 + OddsBar.LABEL_H

    def __init__(self):
        super().__init__()
        self.setWindowFlags(Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.Tool
                            | Qt.WindowTransparentForInput)
        self.setAttribute(Qt.WA_ShowWithoutActivating)
        self.setStyleSheet("background:#15171c; color:#e6e6e6;")
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 3, 6, 4)
        lay.setSpacing(2)
        self.title = QLabel()
        self.title.setAlignment(Qt.AlignCenter)
        lay.addWidget(self.title)
        self.bar = OddsBar(0, 0, 100, height=18)
        lay.addWidget(self.bar)
        self.resize(self.WIDTH, self.HEIGHT)

    def show_combat(self, combat):
        """Met à jour le bandeau ; renvoie False s'il n'y a rien à montrer."""
        now = (combat or {}).get("now")
        if not now:
            return False
        who = html.escape(now.get("opponent") or "opponent")
        kind = "COMBAT" if now["live"] else "NEXT FIGHT"
        extra = ""
        if now["lethal_me"] >= 0.5:
            extra = f" <b style='color:{LOSS}'>☠ {now['lethal_me']:.0f}%</b>"
        elif now["lethal"] >= 0.5:
            extra = f" <b style='color:{WIN}'>lethal {now['lethal']:.0f}%</b>"
        self.title.setText(f"<b style='color:#c9a227'>{kind}</b> <span style='color:#9aa0ad'>vs {who}</span>"
                           + extra)
        self.bar.parts = [(now["win"], WIN), (now["tie"], TIE), (now["loss"], LOSS)]
        self.bar.update()
        return True


def top_overlay_geometry(rect):
    """Centré en haut du jeu, juste au-dessus du portrait de l'adversaire."""
    x, y, w, h = rect
    width, height = CombatOverlay.WIDTH, CombatOverlay.HEIGHT
    return int(x + (w - width) / 2), int(y + h * 0.005), width, height
