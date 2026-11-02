"""Base de cartes : nom, tier (techLevel) et races de chaque carte, depuis HearthstoneJSON.

Qui fait quoi :
- HearthstoneJSON publie toutes les cartes du jeu dans un fichier JSON gratuit.
- download() le récupère dans cards.json (dossier de mes données, USER_DIR) ; get_db() le charge une seule fois en mémoire.
- data/cards-fallback.json.gz : une copie réduite incluse dans l'appli. Premier lancement sans internet (ou
  HearthstoneJSON en panne) : l'appli démarre avec elle au lieu de planter.
- Mise à jour en arrière-plan (jamais dans le fil de l'interface) : au démarrage si cards.json manque ou a
  plus de 7 jours, et quand le jeu montre une carte inconnue (nouveau patch). La base neuve remplace
  l'ancienne d'un coup, une fois chargée.
- Card : petit objet lisible (card.name, card.tier, card.races) au lieu d'un dict brut.
"""
import gzip
import json                    # lire le fichier cards.json
import os
import re
import threading
import time
import urllib.request          # télécharger sans dépendance externe
from dataclasses import dataclass

from .paths import DATA_DIR, USER_DIR

CARDS_URL = "https://api.hearthstonejson.com/v1/latest/enUS/cards.json"
CARDS_PATH = USER_DIR / "cards.json"
FALLBACK_PATH = DATA_DIR / "cards-fallback.json.gz"
MAX_AGE = 7 * 86400            # cards.json plus vieux : on le retélécharge (patchs fréquents)
UNKNOWN_RETRY = 6 * 3600       # carte inconnue : au plus un téléchargement toutes les 6 h
NO_DOWNLOAD = "OWLINGSTONE_NO_CARD_DOWNLOAD"   # tests : jamais de réseau
CARD_ID = re.compile(r"^[A-Za-z0-9]+_[A-Za-z0-9_]+$")

# Races sorties du pool alors que HearthstoneJSON les marque encore "pool" (à mettre à jour à
# chaque saison). Naga : retirés au patch 36.6.1 (22/09/2026) ; les doubles races Naga restent.
ROTATED_OUT_RACES = {"NAGA"}


@dataclass(frozen=True)
class Card:
    id: str                 # identifiant technique, ex. "BG20_100"
    dbf_id: int             # identifiant numérique (sert pour relier version dorée/normale)
    name: str               # nom affiché, ex. "Razorfen Geomancer"
    tier: int | None        # tier de taverne 1 à 7 (None si pas un serviteur BG)
    races: tuple            # ex. ("QUILBOAR",) ; ("ALL",) pour les Amalgames
    is_pool_minion: bool    # True si la carte peut apparaître dans la taverne de Bob
    is_hero: bool           # True si c'est un héros jouable en Battlegrounds
    text: str               # texte de la carte (pour repérer Râle d'agonie, etc.)
    attack: int = 0
    health: int = 0
    cost: int = 0           # coût en or (utile pour trier les sorts de taverne)
    is_pool_spell: bool = False  # sort de taverne vendu par Bob
    is_duos_only: bool = False   # n'existe qu'en Duos (jamais proposé en Solo)
    keywords: tuple = ()         # mots-clés du jeu qu'elle a ou cite (DEATHRATTLE, BACON_RALLY...)


def download(path=CARDS_PATH):
    """Télécharge (ou remplace) cards.json. À relancer après un patch du jeu."""
    from . import __version__
    path.parent.mkdir(parents=True, exist_ok=True)
    # HearthstoneJSON refuse (403) le User-Agent par défaut de Python : on se présente.
    req = urllib.request.Request(CARDS_URL, headers={
        "User-Agent": f"Owlingstone/{__version__} (+https://github.com/owlingstone/owlingstone)"})
    tmp = path.with_suffix(".part")
    with urllib.request.urlopen(req, timeout=60) as resp:  # nosec B310 : URL fixe https
        data = resp.read(100_000_001)
    cards = json.loads(data)     # refuse un fichier coupé ou une page d'erreur
    if not isinstance(cards, list) or len(cards) < 1000 or len(data) > 100_000_000:
        raise ValueError("HearthstoneJSON sent something that is not the card list")
    tmp.write_bytes(data)
    tmp.replace(path)            # remplace seulement si le téléchargement est complet


class CardDb:
    def __init__(self, raw_cards):
        self.by_id = {}          # "BG20_100" -> Card
        self.by_dbf = {}         # 12345 -> Card
        for c in raw_cards:
            card = Card(
                id=c["id"],
                dbf_id=c.get("dbfId", 0),
                name=c.get("name", c["id"]),
                tier=c.get("techLevel"),
                races=tuple(c.get("races") or ([c["race"]] if c.get("race") else [])),
                is_pool_minion=bool(c.get("isBattlegroundsPoolMinion")),
                is_hero=bool(c.get("battlegroundsHero")),
                text=c.get("text", ""),
                attack=c.get("attack", 0),
                health=c.get("health", 0),
                cost=c.get("cost", 0),
                is_pool_spell=bool(c.get("isBattlegroundsPoolSpell")),
                is_duos_only=bool(c.get("isBattlegroundsDuosExclusive")),
                keywords=tuple(c.get("mechanics", []) + c.get("referencedTags", [])),
            )
            self.by_id[card.id] = card
            self.by_dbf[card.dbf_id] = card
        # Version dorée -> version normale : "battlegroundsNormalDbfId" pointe vers la normale.
        self.normal_of = {
            c["id"]: self.by_dbf[c["battlegroundsNormalDbfId"]].id
            for c in raw_cards
            if c.get("battlegroundsNormalDbfId") in self.by_dbf
        }

    def get(self, card_id):
        card = self.by_id.get(card_id)
        if card is None and card_id:
            _unknown(card_id)
        return card

    def base(self, card_id):
        """Carte normale (non dorée) correspondant à card_id."""
        card = self.by_id.get(self.normal_of.get(card_id, card_id))
        if card is None and card_id:
            _unknown(card_id)
        return card

    def pool_minions(self, duos=False):
        """Serviteurs que Bob peut proposer : sans les cartes Duos (en Solo) ni les races sorties."""
        return [c for c in self.by_id.values() if c.is_pool_minion and _in_mode(c, duos)
                and not (c.races and set(c.races) <= ROTATED_OUT_RACES)]

    def pool_spells(self, duos=False):
        return [c for c in self.by_id.values() if c.is_pool_spell and _in_mode(c, duos)]


def _in_mode(card, duos):
    """Une carte Duos n'existe pas en Solo ; en Duos, tout le pool est possible."""
    return duos or not card.is_duos_only


# ------------------------------------------------------------------ chargement et mise à jour
_lock = threading.Lock()
_db = None
_state = {"thread": None, "last_try": 0.0, "source": None}


def _read(path):
    opener = gzip.open if path.suffix == ".gz" else open
    with opener(path, "rt", encoding="utf-8") as f:
        return CardDb(json.load(f))


def get_db():
    """La base de cartes, chargée une fois : cards.json de l'utilisateur, sinon la copie incluse.
    Ne télécharge jamais dans le fil appelant (l'interface ne gèle pas, et hors ligne rien ne plante)."""
    global _db
    db = _db
    if db is not None:
        return db
    with _lock:
        if _db is None:
            try:
                _db, _state["source"] = _read(CARDS_PATH), CARDS_PATH
            except (OSError, ValueError, KeyError, TypeError):
                _db, _state["source"] = _read(FALLBACK_PATH), FALLBACK_PATH
            if _stale():
                refresh_in_background()
        return _db


def reset():
    """Oublie la base en mémoire : le prochain get_db() relit le fichier (après un téléchargement)."""
    global _db
    with _lock:
        _db = None


def _stale(now=None):
    now = time.time() if now is None else now
    try:
        return now - CARDS_PATH.stat().st_mtime > MAX_AGE
    except OSError:
        return True


def refresh_in_background(force=False, now=None):
    """Télécharge cards.json dans un fil à part, au plus une fois toutes les UNKNOWN_RETRY secondes.
    Renvoie le fil lancé, ou None si rien à faire."""
    now = time.time() if now is None else now
    if os.environ.get(NO_DOWNLOAD):
        return None
    t = _state["thread"]
    if t is not None and t.is_alive():
        return None
    if not force and now - _state["last_try"] < UNKNOWN_RETRY:
        return None
    _state["last_try"] = now

    def work():
        global _db
        try:
            download()
            fresh = _read(CARDS_PATH)       # chargée ici, pas dans le fil de l'interface
            with _lock:
                _db, _state["source"] = fresh, CARDS_PATH
        except (OSError, ValueError, KeyError, TypeError):
            pass                  # hors ligne ou site en panne : on garde la base actuelle
    t = threading.Thread(target=work, name="cards-refresh", daemon=True)
    _state["thread"] = t
    t.start()
    return t


def _unknown(card_id):
    """Le jeu montre une carte absente de la base : sûrement un nouveau patch. On retélécharge, sauf si
    cards.json vient d'être téléchargé (certains ids du log n'existent nulle part)."""
    if not CARD_ID.match(str(card_id)):
        return
    try:
        fresh = time.time() - CARDS_PATH.stat().st_mtime < UNKNOWN_RETRY
    except OSError:
        fresh = False
    if not fresh:
        refresh_in_background()
