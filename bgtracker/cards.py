"""Base de cartes : nom, tier (techLevel) et races de chaque carte, depuis HearthstoneJSON.

Qui fait quoi :
- HearthstoneJSON publie toutes les cartes du jeu dans un fichier JSON gratuit.
- download() le récupère dans cards.json (dossier de mes données, USER_DIR) ; get_db() le charge une seule fois en mémoire.
- Card : petit objet lisible (card.name, card.tier, card.races) au lieu d'un dict brut.
"""
import json                    # lire le fichier cards.json
import urllib.request          # télécharger sans dépendance externe
from dataclasses import dataclass
from functools import lru_cache  # mémorise le résultat : on ne charge le fichier qu'une fois

from .paths import USER_DIR

CARDS_URL = "https://api.hearthstonejson.com/v1/latest/enUS/cards.json"
CARDS_PATH = USER_DIR / "cards.json"

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
    path.parent.mkdir(parents=True, exist_ok=True)
    urllib.request.urlretrieve(CARDS_URL, path)


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
        return self.by_id.get(card_id)

    def base(self, card_id):
        """Carte normale (non dorée) correspondant à card_id."""
        return self.by_id.get(self.normal_of.get(card_id, card_id))

    def pool_minions(self, duos=False):
        """Serviteurs que Bob peut proposer : sans les cartes Duos (en Solo) ni les races sorties."""
        return [c for c in self.by_id.values() if c.is_pool_minion and _in_mode(c, duos)
                and not (c.races and set(c.races) <= ROTATED_OUT_RACES)]

    def pool_spells(self, duos=False):
        return [c for c in self.by_id.values() if c.is_pool_spell and _in_mode(c, duos)]


def _in_mode(card, duos):
    """Une carte Duos n'existe pas en Solo ; en Duos, tout le pool est possible."""
    return duos or not card.is_duos_only


@lru_cache(maxsize=1)
def get_db():
    """Charge la base une fois ; la télécharge si le fichier n'existe pas encore."""
    if not CARDS_PATH.exists():
        download()
    with open(CARDS_PATH, encoding="utf-8") as f:
        return CardDb(json.load(f))
