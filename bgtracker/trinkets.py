"""Trinkets : lequel prendre dans l'offre, et comment orienter l'offre suivante avec son plateau.

Règles des offres (Blizzard, "Developer Insights: Updated Trinket Rules" + wiki, toujours vraies en saison 14) :
- 2 offres par partie, au début du tour 6 (Lesser) et du tour 9 (Greater) : 4 Trinkets au choix, on en
  achète un (son prix en or est payé ce tour-là, donc moins d'or pour les serviteurs).
- L'offre dépend des races de la partie, de mon plateau, de ma main et de mon pouvoir héroïque.
- Un Trinket "typé" (lié à une race) demande d'être "dans" cette race : au moins 2 serviteurs de la race au
  tour 6, 3 au tour 9 (en avoir plus n'augmente pas les chances). 3 races différentes = "menagerie".
- Chaque offre contient : un Trinket de ma race la plus présente, au moins un Trinket sans race et au moins un à
  2 d'or ou moins. Au plus un Trinket d'une race où je ne suis pas : celui-là coûte 2 de moins ("pivot").
- Certains héros sont toujours "dans" une race ; certains Trinkets demandent un serviteur précis (Râle d'agonie,
  Bouclier divin, doré, Début de combat, 6 serviteurs...).
=> Le plateau (et la main) à la FIN du tour 5 / du tour 8 décide des Trinkets proposés : next_offer_hint()
   dit quoi garder pour viser une race.

Choix (rank_offer) : point de départ = placement moyen des joueurs qui ont pris ce Trinket (bgbuddy.gg, patch
actuel, data/trinkets.yaml). Ce chiffre vient de joueurs qui l'ont pris quand il leur allait : on le corrige
quand il ne me va PAS (race absente de mon plateau et de mes comps, mécanique absente), et un peu quand il va
très bien (serviteur donné qui est une carte de ma comp, race de ma comp, synergie avec mon plateau / pouvoir).
"""
import math
import re
from collections import Counter
from dataclasses import dataclass, field

from .cards import get_db
from .paths import DATA_DIR
from .planner import RACE_WORDS, _load_yaml, _mentions_race, board_synergy

TRINKETS_PATH = DATA_DIR / "trinkets.yaml"

OFFER_TURNS = {6: "lesser", 9: "greater"}       # tour de l'offre -> genre de Trinket
IN_TYPE = {"lesser": 2, "greater": 3}           # serviteurs d'une race pour être "dans" la race
PRIOR_AVG, PRIOR_GAMES = 3.95, 500              # petit échantillon : tiré vers un Trinket moyen
UNKNOWN_AVG = 4.0                               # Trinket absent des stats

# Corrections, en places (plus = pire). Les stats valent déjà "quand il va" : les malus sont plus forts que les bonus.
OFF_TYPE = 0.45          # race du Trinket : aucun serviteur à moi, pas dans mes comps
WEAK_TYPE = 0.15         # race : 1 seul serviteur (ou seulement dans la boutique)
COMP_TYPE = -0.10        # race d'une comp que je vise
GIVES_CORE = -0.30       # le serviteur donné est une carte core de ma comp
GIVES_ADDON = -0.12      # ... une carte add-on
GIVES_OFF = 0.25         # le serviteur donné est d'une race que je ne joue pas du tout
NO_MECHANIC = 0.30       # demande une mécanique (Râle d'agonie, Bouclier divin...) que je n'ai pas
SYNERGY_STEP = -0.05     # par point de board_synergy (0,6 par serviteur qui va avec)
SYNERGY_CAP = -0.15
PIVOT_GOLD = -0.04       # par pièce d'or économisée (pivot -2)

# Mécanique demandée par le texte d'un Trinket -> ce qu'un serviteur doit avoir pour en profiter.
NEEDS = {
    "Deathrattle": (r"\byour Deathrattle|Deathrattle minions?|your minions?' Deathrattles|friendly Deathrattle",
                    r"Deathrattle"),
    "Divine Shield": (r"(your|friendly)[^.]*Divine Shield|minions? with Divine Shield", r"Divine Shield"),
    "Battlecry": (r"\bBattlecry minions?|your Battlecr|Battlecries", r"^(<b>)?Battlecry|\bBattlecry:"),
    "Start of Combat": (r"your Start of Combat|Start of Combat minions?", r"Start of Combat"),
    "End of Turn": (r"your End of Turn|End of Turn minions?", r"end of (your )?turn"),
    "Avenge": (r"\bAvenge minions?|your Avenge", r"Avenge"),
    "Rally": (r"\bRally minions?|your Rally", r"Rally"),
    "Magnetic": (r"Magnetize", r"Magnetic"),
    "Blood Gem": (r"Blood Gems?", r"Blood Gem"),
}
# (Pas de "Tavern spell" : tout le monde en achète, et les Gemmes de sang sont des sorts.)
_TAGS = re.compile(r"<[^>]+>|\[x\]")


@dataclass
class TrinketPick:
    card: object                 # Card
    cost: int                    # prix réel (après réduction "pivot")
    score: float                 # placement attendu (plus bas = mieux)
    stat: dict | None            # ligne de data/trinkets.yaml
    reasons: list = field(default_factory=list)
    affordable: bool = True


def _clean(text):
    return _TAGS.sub("", text or "").replace("\n", " ")


def load_trinkets(path=TRINKETS_PATH):
    """{id de carte: infos (name, kind, cost, avg, pick, games, races, gives)} (relu s'il a changé)."""
    return _load_yaml(path, lambda raw: {t["id"]: t for t in raw} if isinstance(raw, list) else {})


def race_counts(cards):
    """Serviteurs par race (un Amalgame compte pour toutes, une double race pour les deux)."""
    n = Counter()
    for c in cards:
        if c is None or not c.tier:
            continue
        for r in (RACE_WORDS if "ALL" in c.races else c.races):
            n[r] += 1
    return n


def _base_avg(stat):
    if not stat:
        return UNKNOWN_AVG
    g = stat.get("games", 0)
    return (stat["avg"] * g + PRIOR_AVG * PRIOR_GAMES) / (g + PRIOR_GAMES) if g < 5000 else stat["avg"]


def trinket_races(card, stat):
    """Races liées au Trinket : celles de la carte, sinon celle du serviteur donné, sinon citées dans son texte."""
    if stat and stat.get("races"):
        return [r for r in stat["races"] if r in RACE_WORDS]
    if stat and stat.get("gives"):
        given = _by_name().get(stat["gives"])
        if given and given.races and "ALL" not in given.races:
            return list(given.races)
    text = _clean(card.text)
    return [r for r in RACE_WORDS if r != "NAGA" and _mentions_race(text, r)]


_name_cache = {}


def _by_name():
    db = get_db()
    if _name_cache.get("db") is not db:
        _name_cache["db"] = db
        _name_cache["map"] = {c.name: c for c in db.pool_minions(duos=True)}
    return _name_cache["map"]


def rank_offer(offer, board=(), hand=(), shop=(), targets=(), powers=(), gold=None, table=None):
    """Classe les Trinkets proposés, du meilleur au moins bon.

    offer   : [(Card, prix réel)] ; board / hand / shop : Cards ; targets : [(Comp, poids)] (advice.target_comps) ;
    powers  : Cards de mes pouvoirs héroïques ; gold : mon or (None = inconnu)."""
    table = load_trinkets() if table is None else table
    mine = [c for c in list(board) + list(hand) if c is not None]
    counts = race_counts(mine)
    shop_counts = race_counts(shop)
    comp_races = {r for comp, w in targets if w >= 0.5 for r in comp.races}
    core = {c.name: comp.name for comp, w in targets if w >= 0.5 for c in comp.core}
    addons = {c.name: comp.name for comp, w in targets if w >= 0.5 for c in comp.addons}
    picks = []
    for card, cost in offer:
        stat = table.get(card.id)
        score, why = _base_avg(stat), []
        if stat:
            why.append(f"avg {stat['avg']:.2f}" + (" (few games)" if stat["games"] < 500 else ""))
        else:
            why.append("no stats")
        races = trinket_races(card, stat)
        if races:
            have = max(counts[r] for r in races)
            word = "/".join(RACE_WORDS[r] for r in races)
            if any(r in comp_races for r in races):
                score += COMP_TYPE
                why.append(f"{word}: your comp")
            elif have >= 2:
                why.append(f"{word}: {have} on board")
            elif have == 1 or any(shop_counts[r] for r in races):
                score += WEAK_TYPE
                why.append(f"{word}: only {have} on board" if have else f"{word}: none yet (some in shop)")
            else:
                score += OFF_TYPE
                why.append(f"no {word} — pivot only")
        gives = (stat or {}).get("gives")
        if gives in core:
            score += GIVES_CORE
            why.append(f"gives {gives} (core of {core[gives]})")
        elif gives in addons:
            score += GIVES_ADDON
            why.append(f"gives {gives} ({addons[gives]})")
        elif gives and races and not any(counts[r] for r in races) and not any(r in comp_races for r in races):
            score += GIVES_OFF
        text = _clean(card.text)
        for name, (wants, has) in NEEDS.items():
            if re.search(wants, text, re.I) and not any(re.search(has, _clean(c.text), re.I) for c in mine + list(powers)):
                if name == "Blood Gem" and counts["QUILBOAR"]:
                    continue
                score += NO_MECHANIC
                why.append(f"no {name} on board")
                break
        pts, syn = board_synergy(card, mine + list(powers))
        if pts:
            score += max(SYNERGY_CAP, SYNERGY_STEP * pts)
            why.append(syn)
        base_cost = (stat or {}).get("cost", card.cost)
        if cost is not None and base_cost and cost < base_cost:
            score += PIVOT_GOLD * (base_cost - cost)
            why.append(f"pivot: {cost} gold instead of {base_cost}")
        pick = TrinketPick(card, cost if cost is not None else base_cost, round(score, 2), stat, why)
        if gold is not None and pick.cost > gold:
            pick.affordable = False
            why.append(f"costs {pick.cost}, you have {gold}")
        picks.append(pick)
    picks.sort(key=lambda p: (not p.affordable, p.score))
    return picks


def next_offer_hint(turn, board=(), hand=(), lobby_races=None, targets=(), table=None, top=2):
    """Avant une offre (tours 5 et 8 : l'offre arrive au début du tour suivant) : quelles races mon plateau + ma
    main débloquent, et quoi ajouter pour en débloquer une autre. None aux autres tours."""
    kind = OFFER_TURNS.get(turn + 1)
    if kind is None:
        return None
    table = load_trinkets() if table is None else table
    need = IN_TYPE[kind]
    counts = race_counts([c for c in list(board) + list(hand) if c is not None])
    comp_races = [r for comp, w in targets if w >= 0.5 for r in comp.races]
    lobby = set(lobby_races) if lobby_races else set(RACE_WORDS) - {"NAGA"}
    best = {}
    for t in table.values():
        if t["kind"] != kind:
            continue
        for r in t.get("races") or ():
            best.setdefault(r, []).append(t)
    for r in best:
        best[r].sort(key=lambda t: _base_avg(t))

    def names(race):
        return ", ".join(f"{t['name']} {t['avg']:.2f}" for t in best.get(race, [])[:top])

    unlocked = [r for r in RACE_WORDS if counts[r] >= need and r in lobby]
    close = [r for r in RACE_WORDS if counts[r] == need - 1 and r in lobby and r not in unlocked]
    close.sort(key=lambda r: (r not in comp_races, _base_avg(best.get(r, [{}])[0]) if best.get(r) else 9))
    main = max(counts, key=counts.get) if counts else None
    lines = []
    lines.append(f"{kind.title()} Trinkets next turn: end this turn with {need}+ of a tribe (board + hand) "
                 "to be offered its Trinkets.")
    for r in unlocked:
        lines.append(f"✓ {RACE_WORDS[r]} ({counts[r]}): {names(r) or 'no stats'}")
    close = [r for r in close if best.get(r)]
    for r in close[:2]:
        tag = " — your comp" if r in comp_races else ""
        lines.append(f"+1 {RACE_WORDS[r]} unlocks: {names(r) or 'no stats'}{tag}")
    if len([r for r in counts if counts[r]]) >= 3:
        lines.append("3+ tribes: menagerie Trinkets possible too.")
    if main and counts[main] >= 1:
        lines.append(f"Guaranteed: one {RACE_WORDS[main]} Trinket (your main tribe).")
    return {"kind": kind, "need": need, "lines": lines, "unlocked": unlocked, "close": close[:2]}
