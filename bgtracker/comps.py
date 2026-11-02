"""Compositions : charger data/comps.yaml et deviner "ce que je joue" d'après mon plateau.

Qui fait quoi :
- load_comps() lit le fichier YAML (modifiable à la main) et relie chaque nom de carte à sa Card.
- available_comps(races) garde les comps dont toutes les races sont dans la partie (comme HDT,
  qui ne propose que les guides jouables avec les races du lobby).
- board_profile(cards) résume le plateau : nombre de serviteurs par race et par mot-clé.
- match_comps(...) note chaque comp : 3 points par carte "core" possédée, 1 par "addon",
  + un bonus selon la part du plateau qui est des bonnes races. On renvoie les meilleures,
  avec les cartes déjà possédées et celles qui manquent (base des futures recommandations).
- Les règles de data/rules.yaml (rules.py) cachent des comps ou ajoutent/retirent des points.
- lobby_comps(...) : comps proposées en début de partie, triées par règles puis placement moyen.
"""
from dataclasses import dataclass, field
from pathlib import Path

import yaml  # PyYAML : lit le format YAML (plus lisible que le JSON pour un humain)

from .cards import get_db
from .paths import DATA_DIR
from .rules import judge

COMPS_PATH = DATA_DIR / "comps.yaml"

# Mots-clés repérés dans le texte des cartes pour décrire le plateau ("Deathrattle ×3")
KEYWORDS = ["Deathrattle", "Battlecry", "Rally", "Spellcraft", "Magnetic", "Avenge", "Divine Shield",
            "Reborn", "Taunt", "Blood Gem", "Tavern spell", "Discard", "Deity", "Golden", "End of your turn"]

CORE_POINTS, ADDON_POINTS, RACE_BONUS = 3, 1, 4


@dataclass
class Comp:
    id: str
    name: str
    races: list
    core: list            # liste de Card
    addons: list          # liste de Card
    difficulty: str = ""
    when_to_commit: str = ""
    how_to_play: str = ""
    stats: dict = field(default_factory=dict)   # {"avg": placement moyen, "games": nb de parties}


@dataclass
class CompMatch:
    comp: Comp
    score: float          # points obtenus
    confidence: int       # 0-100 : score / score maximal possible
    have: list = field(default_factory=list)      # cartes de la comp que je possède
    missing_core: list = field(default_factory=list)
    missing_addons: list = field(default_factory=list)
    rule_bonus: float = 0                         # points ajoutés/retirés par les règles
    notes: list = field(default_factory=list)     # notes des règles appliquées


def _card_by_name(name):
    """Carte du pool (serviteur ou sort) portant ce nom exact, ou None."""
    db = get_db()
    for c in db.pool_minions() + db.pool_spells():
        if c.name == name:
            return c
    return None


def load_comps(path=COMPS_PATH):
    """Lit comps.yaml. Lève une erreur claire si un nom de carte n'existe pas (faute de frappe)."""
    with open(path, encoding="utf-8") as f:
        raw = yaml.safe_load(f) or []
    comps, unknown = [], []
    for r in raw:
        cards = {}
        for key in ("core", "addons"):
            cards[key] = []
            for name in r.get(key, []):
                card = _card_by_name(name)
                if card is None:
                    unknown.append(f"{r['id']}: {name}")
                else:
                    cards[key].append(card)
        comps.append(Comp(id=r["id"], name=r["name"], races=r.get("races", []),
                          core=cards["core"], addons=cards["addons"],
                          difficulty=r.get("difficulty", ""), when_to_commit=r.get("when_to_commit", ""),
                          how_to_play=r.get("how_to_play", ""), stats=r.get("stats") or {}))
    if unknown:
        raise ValueError("Cartes inconnues dans comps.yaml : " + "; ".join(unknown))
    return comps


def available_comps(comps, races):
    """Comps jouables avec ces races (races=None : pas encore détectées -> toutes)."""
    if races is None:
        return list(comps)
    return [c for c in comps if all(r in races for r in c.races)]


def board_profile(cards):
    """Résumé du plateau : {"races": {race: n}, "keywords": {mot: n}} (cartes = liste de Card)."""
    races, keywords = {}, {}
    for c in cards:
        for r in c.races:
            if r != "ALL":
                races[r] = races.get(r, 0) + 1
        for k in KEYWORDS:
            if k.lower() in (c.text or "").lower():
                keywords[k] = keywords.get(k, 0) + 1
    return {"races": races, "keywords": keywords}


HAND_WEIGHT = 0.5  # une carte en main compte moitié : c'est le plateau qui dit ce que je joue


def match_comps(comps, board, hand=(), races=None, top=3, rules=(), deity=None, hero=None, min_confidence=0,
                hero_power=None):
    """Classe les comps disponibles selon mes cartes (board/hand = listes de Card) et mes règles.

    min_confidence : écarte les comps trop peu reconnues (en %) avant de garder les 'top' premières."""
    weight = {c.id: HAND_WEIGHT for c in hand}
    weight.update({c.id: 1.0 for c in board})        # sur le plateau : poids plein
    owned = set(weight)
    minions = [c for c in board if not c.is_pool_spell]   # la "couleur" du plateau
    ctx = {"races": races, "deity": deity, "hero": hero, "hero_power": hero_power}
    results = []
    for comp in available_comps(comps, races):
        verdict = judge(comp.id, rules, ctx)
        if verdict.hidden:
            continue
        have = [c for c in comp.core + comp.addons if c.id in owned]
        core_hits = sum(weight.get(c.id, 0) for c in comp.core)
        addon_hits = sum(weight.get(c.id, 0) for c in comp.addons)
        # Part de mes serviteurs qui appartiennent aux races de la comp (0 à 1)
        fit = (sum(1 for c in minions if set(c.races) & set(comp.races) or "ALL" in c.races) / len(minions)
               if minions else 0)
        score = CORE_POINTS * core_hits + ADDON_POINTS * addon_hits + RACE_BONUS * fit
        best = CORE_POINTS * min(len(comp.core), 3) + ADDON_POINTS * min(len(comp.addons), 3) + RACE_BONUS
        results.append(CompMatch(
            comp=comp, score=score, confidence=min(100, round(100 * score / best)), have=have,
            missing_core=[c for c in comp.core if c.id not in owned],
            missing_addons=[c for c in comp.addons if c.id not in owned],
            rule_bonus=verdict.bonus, notes=verdict.notes))
    # Ce que je joue, c'est mon plateau qui le dit : les règles ne font que départager deux comps à
    # égalité (elles comptent pleinement dans lobby_comps). Une comp sans aucune de mes cartes n'est
    # jamais "jouée" : la seule part de bonnes races sur le plateau ne suffit pas.
    results.sort(key=lambda m: (m.score, m.rule_bonus), reverse=True)
    return [m for m in results if m.have and m.confidence >= min_confidence][:top]


HERO_COMPS_PATH = DATA_DIR / "hero_comps.yaml"
_hero_comps = {}


def load_hero_comps(path=HERO_COMPS_PATH):
    """{id de comp: {héros: (placement moyen avec cette comp, parties)}} : meilleurs héros de chaque comp
    (bgbuddy.gg, patch actuel). Relu s'il change."""
    path = Path(path)
    if not path.exists():
        return {}
    mtime = path.stat().st_mtime
    if _hero_comps.get("mtime") != mtime:
        with open(path, encoding="utf-8") as f:
            raw = yaml.safe_load(f) or {}
        _hero_comps.update(mtime=mtime, table={cid: {h: tuple(v) for h, v in (heroes or {}).items()}
                                               for cid, heroes in raw.items()})
    return _hero_comps["table"]


def comp_avg(comp, hero=None, table=None):
    """Placement moyen de la comp : celui de mon héros s'il fait partie de ses meilleurs héros, sinon
    celui de la comp (9 sans stats)."""
    table = load_hero_comps() if table is None else table
    found = (table.get(comp.id) or {}).get(hero) if hero else None
    return found[0] if found else comp.stats.get("avg", 9)


# Tier list : placement moyen ajusté (mon héros, mes règles) -> lettre. Une règle "prefer_if" de 3 points
# (= une carte core) gagne 0,15 place ; "avoid_if" en perd autant.
TIERS = [("S", 3.30), ("A", 3.50), ("B", 3.70), ("C", 3.90), ("D", 99)]
RULE_PLACE = 0.05


def comp_tier(comp, hero=None, bonus=0, table=None):
    """(lettre, placement ajusté) de la comp dans ce lobby ; ("?", 9) sans stats."""
    avg = round(comp_avg(comp, hero, table) - RULE_PLACE * bonus, 6)   # 3.45 - 0.15 = 3.3000000000000003
    if avg >= 9:
        return "?", avg
    return next(letter for letter, top in TIERS if avg <= top), avg


def hero_best_comps(hero, comps, races=None, top=3, table=None):
    """[(Comp, placement moyen du héros avec elle, parties)] : les comps où ce héros est parmi les
    meilleurs, jouables dans ce lobby (toutes si les races ne sont pas connues), meilleures d'abord."""
    table = load_hero_comps() if table is None else table
    found = [(c, *table[c.id][hero]) for c in available_comps(comps, races)
             if hero in (table.get(c.id) or {})]
    return sorted(found, key=lambda x: x[1])[:top]


def lobby_comps(comps, races=None, rules=(), deity=None, hero=None, hero_power=None):
    """Comps jouables dans ce lobby, sans celles que mes règles cachent : préférées d'abord,
    puis meilleur placement moyen, celui de mon héros quand il fait partie des meilleurs héros de la comp
    (les comps sans stats en dernier)."""
    ctx = {"races": races, "deity": deity, "hero": hero, "hero_power": hero_power}
    judged = [(c, judge(c.id, rules, ctx)) for c in available_comps(comps, races)]
    judged = [(c, v) for c, v in judged if not v.hidden]
    table = load_hero_comps()
    judged.sort(key=lambda cv: (-cv[1].bonus, comp_avg(cv[0], hero, table)))
    return [c for c, _ in judged]


def cards_from_ids(card_ids):
    db = get_db()
    return [db.get(i) for i in card_ids if db.get(i)]
