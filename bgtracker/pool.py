"""Liste des serviteurs par tier, filtrée par les races de la partie (comme le navigateur de HDT).

Logique reprise de HDT (BattlegroundsDb.cs / BattlegroundsMinionsViewModel.cs), réécrite ici :
- Groupes d'un tier : "ALL" (Amalgames) d'abord, puis une section par race, puis "Other"
  (neutres), puis les sorts de taverne triés par coût.
- Un serviteur à double race apparaît dans la section de chacune de ses races présentes.
- Un neutre "lié" à une race n'apparaît que si cette race est dans la partie. Le jeu le signale par
  les tags BACON_SUBSET_<RACE>, absents de HearthstoneJSON : on les APPREND depuis les logs
  (subsets.json, dans mes données). En secours, règle de HDT : un neutre dont le texte cite UNE seule race
  est rangé dans cette race.
"""
import json
import re

from .cards import get_db
from .paths import DATA_DIR, USER_DIR

SUBSETS_PATH = USER_DIR / "subsets.json"          # ce que j'ai appris dans mes parties
SUBSETS_SEED = DATA_DIR / "subsets.json"          # version de départ livrée avec l'application

# Tag du log -> race (les noms ne sont pas toujours identiques : MECH/MECHANICAL, QUILLBOAR/QUILBOAR)
SUBSET_TAG_RACE = {
    "BACON_SUBSET_BEAST": "BEAST", "BACON_SUBSET_DEMON": "DEMON", "BACON_SUBSET_DRAGON": "DRAGON",
    "BACON_SUBSET_ELEMENTALS": "ELEMENTAL", "BACON_SUBSET_MECH": "MECHANICAL",
    "BACON_SUBSET_MURLOC": "MURLOC", "BACON_SUBSET_NAGA": "NAGA", "BACON_SUBSET_PIRATE": "PIRATE",
    "BACON_SUBSET_QUILLBOAR": "QUILBOAR", "BACON_SUBSET_UNDEAD": "UNDEAD",
    "BACON_SUBSET_ABERRATION": "ABERRATION",
}
ALL_RACES = sorted(set(SUBSET_TAG_RACE.values()))

# Nom affiché de chaque race (et mot cherché dans le texte des cartes)
RACE_LABEL = {
    "BEAST": "Beast", "DEMON": "Demon", "DRAGON": "Dragon", "ELEMENTAL": "Elemental",
    "MECHANICAL": "Mech", "MURLOC": "Murloc", "NAGA": "Naga", "PIRATE": "Pirate",
    "QUILBOAR": "Quilboar", "UNDEAD": "Undead", "ABERRATION": "Aberration",
    "ALL": "All types", "NEUTRAL": "Other", "SPELL": "Spells", "VARIES": "Tier changes each game",
}
# Serviteurs dont le tier change à chaque partie (le fichier de cartes donne un tier factice) :
# listés à part, en bas de chaque tier, au lieu d'être rangés dans un seul.
VARIABLE_TIER = {"Dark Paradox"}


# Filtre par mot-clé (comme HDT) : libellé affiché -> tag du jeu que la carte a ou cite
# (ex. Titus Rivendare cite DEATHRATTLE : il sort quand on filtre "Deathrattle").
KEYWORDS = {
    "Battlecry": "BATTLECRY", "Deathrattle": "DEATHRATTLE", "Rally": "BACON_RALLY",
    "End of Turn": "END_OF_TURN_TRIGGER", "Start of Combat": "START_OF_COMBAT",
    "Divine Shield": "DIVINE_SHIELD", "Taunt": "TAUNT", "Reborn": "REBORN", "Magnetic": "MAGNETIC",
    "Avenge": "AVENGE", "Choose One": "CHOOSE_ONE", "Windfury": "WINDFURY", "Venomous": "VENOMOUS",
}


# ------------------------------------------------------------ apprentissage des sous-ensembles
def load_subsets():
    """card_id -> liste des races du sous-ensemble (appris dans les logs)."""
    for path in (SUBSETS_PATH, SUBSETS_SEED):
        if path.exists():
            return json.loads(path.read_text(encoding="utf-8"))
    return {}


def save_subsets(subsets):
    SUBSETS_PATH.parent.mkdir(parents=True, exist_ok=True)
    SUBSETS_PATH.write_text(json.dumps(subsets, indent=1, sort_keys=True), encoding="utf-8")


def learn_subsets(games, subsets=None):
    """Ajoute à 'subsets' les tags BACON_SUBSET vus sur les serviteurs du pool de ces parties."""
    subsets = dict(subsets or {})
    db = get_db()
    for g in games:
        for e in g.entities.values():
            card = db.base(e.card_id) if e.card_id else None
            if not card or not card.is_pool_minion:
                continue
            races = sorted(r for tag, r in SUBSET_TAG_RACE.items() if e.get(tag))
            if races:
                known = set(subsets.get(card.id, []))
                subsets[card.id] = sorted(known | set(races))
    return subsets


# ------------------------------------------------------------------- rangement des cartes
def _races_in_text(card):
    """Races citées dans le texte (règle de secours de HDT pour les neutres)."""
    found = []
    for race, word in RACE_LABEL.items():
        if race in ("ALL", "NEUTRAL", "SPELL"):
            continue
        if re.search(rf"\b{word}s?\b", card.text or ""):
            found.append(race)
    return found


def card_groups(card, subsets):
    """Sections où ranger la carte : ses races, sinon son sous-ensemble, sinon 'NEUTRAL'."""
    if "ALL" in card.races:
        return ["ALL"]
    if card.races:
        return list(card.races)
    if card.id in subsets:
        return list(subsets[card.id])
    text_races = _races_in_text(card)
    if len(text_races) == 1:
        return text_races
    return ["NEUTRAL"]


def is_available(groups, races):
    """La carte peut-elle sortir dans cette partie ? races=None : on ne sait pas encore -> oui."""
    if races is None or groups in (["ALL"], ["NEUTRAL"]):
        return True
    return any(g in races for g in groups)


def available_keywords(races=None, subsets=None, duos=False):
    """Mots-clés (libellés de KEYWORDS) portés par au moins un serviteur possible dans cette partie."""
    subsets = load_subsets() if subsets is None else subsets
    tags = {k for c in get_db().pool_minions(duos) if is_available(card_groups(c, subsets), races)
            for k in c.keywords}
    return [label for label, tag in KEYWORDS.items() if tag in tags]


def tier7_reachable(races=None, subsets=None, duos=False, seen_tier7=False):
    """Le tier 7 vaut-il la peine d'être montré ? Oui avant de connaître les races, si une carte de
    tier 7 est déjà apparue (boutique, plateau, main : héros, trinket ou quête qui en donnent), ou si un
    serviteur possible dans ce lobby fait découvrir un tier 7 (Greedy Conniver, donc Pirates)."""
    if races is None or seen_tier7:
        return True
    subsets = load_subsets() if subsets is None else subsets
    return any("Tier 7" in (c.text or "") and is_available(card_groups(c, subsets), races)
               for c in get_db().pool_minions(duos))


def tier_list(tier, races=None, subsets=None, duos=False, keyword=None):
    """Sections du tier demandé : liste de (code_section, [Card triées]).

    races : races sûres de la partie (None = toutes, avant détection) ; duos : partie Duos ;
    keyword : libellé de KEYWORDS pour ne garder que les serviteurs qui l'ont (les sorts sont alors cachés)."""
    db = get_db()
    subsets = load_subsets() if subsets is None else subsets
    sections = {}
    for card in db.pool_minions(duos):
        if keyword and KEYWORDS[keyword] not in card.keywords:
            continue
        if card.name in VARIABLE_TIER:
            sections.setdefault("VARIES", []).append(card)
            continue
        if card.tier != tier:
            continue
        groups = card_groups(card, subsets)
        if not is_available(groups, races):
            continue
        for g in groups:
            if g in ("ALL", "NEUTRAL") or races is None or g in races:
                sections.setdefault(g, []).append(card)
    spells = sorted((c for c in db.pool_spells(duos) if c.tier == tier), key=lambda c: (c.cost, c.name))

    def order(key):  # même ordre que HDT : ALL, races (alphabétique), Other, puis "tier variable", Spells
        return ({"ALL": 0, "NEUTRAL": 2, "VARIES": 3}.get(key, 1), RACE_LABEL.get(key, key))

    result = [(k, sorted(v, key=lambda c: c.name)) for k, v in sorted(sections.items(), key=lambda kv: order(kv[0]))]
    if spells and not keyword:
        result.append(("SPELL", spells))
    return result
