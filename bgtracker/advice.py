"""Conseils (phase 5) : quel héros choisir, quoi acheter dans la boutique.

Qui fait quoi :
- load_heroes() lit data/heroes.yaml (placement moyen de chaque héros, données communautaires).
- rank_heroes(names, heroes) classe les héros proposés au choix, du meilleur au moins bon.
- target_comps(...) : les comps que je vise. Si mon plateau dit déjà ce que je joue, ce sont les
  comps reconnues ; sinon toutes les comps du lobby, les meilleures (mes règles, puis placement
  moyen) avec plus de poids.
- non_golden(...) retire mes cartes dorées avant de compter les triples.
- shop_advice(...) note chaque serviteur de la boutique : 3 s'il est "core" d'une comp visée,
  1 si c'est un add-on (× le poids de la comp), +3 s'il fait un triple, +1 s'il fait une paire.
"""
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path

import yaml

from .comps import ADDON_POINTS, CORE_POINTS, lobby_comps, match_comps
from .paths import DATA_DIR

HEROES_PATH = DATA_DIR / "heroes.yaml"

TRIPLE_POINTS, PAIR_POINTS = 3, 1
PLAYING_MIN_CONFIDENCE = 15   # même seuil que le panneau "Playing" de la fenêtre
LOBBY_TOP = 3                 # en début de partie : les 3 meilleures comps du lobby comptent plus
LOBBY_WEIGHTS = (0.5, 0.25)   # ... (poids réduits : rien n'est encore décidé), les autres moins


def load_heroes(path=HEROES_PATH):
    """{nom du héros: {"avg": placement moyen, "games": nb de parties}} (fichier absent = {})."""
    path = Path(path)
    if not path.exists():
        return {}
    with open(path, encoding="utf-8") as f:
        return yaml.safe_load(f) or {}


def rank_heroes(names, heroes):
    """[(nom, stats ou None)] du meilleur placement moyen au moins bon ; sans chiffre en dernier."""
    return sorted(((n, heroes.get(n)) for n in names), key=lambda ns: ns[1]["avg"] if ns[1] else 9)


def target_comps(comps, board, hand=(), races=None, rules=(), deity=None, hero=None, hero_power=None):
    """[(Comp, poids)] : les comps que mes achats doivent servir."""
    game = {"rules": rules, "deity": deity, "hero": hero, "hero_power": hero_power}
    playing = match_comps(comps, board, hand, races, top=2, min_confidence=PLAYING_MIN_CONFIDENCE, **game)
    if playing:
        return [(m.comp, 1.0 if i == 0 else 0.5) for i, m in enumerate(playing)]
    return [(c, LOBBY_WEIGHTS[0] if i < LOBBY_TOP else LOBBY_WEIGHTS[1])
            for i, c in enumerate(lobby_comps(comps, races, **game))]


def non_golden(cards, golden_ids):
    """Mes cartes sans les dorées (une dorée ne compte plus pour un triple). golden_ids peut
    contenir plusieurs fois le même id : un par exemplaire doré."""
    golden, kept = list(golden_ids), []
    for c in cards:
        if c.id in golden:
            golden.remove(c.id)
        else:
            kept.append(c)
    return kept


@dataclass
class ShopPick:
    card: object                                  # Card de la boutique
    value: float
    reasons: list = field(default_factory=list)   # ex. ["core Mech APM", "triple"]


def shop_advice(shop, owned, targets):
    """Serviteurs de la boutique qui valent le coup, du meilleur au moins bon.

    shop  : Cards proposées par Bob ; owned : mes Cards non dorées (plateau + main), pour les triples ;
    targets : [(Comp, poids)] venant de target_comps."""
    copies = Counter(c.id for c in owned)
    picks = []
    for card in shop:
        value, reasons = 0.0, []
        # La meilleure raison "comp" seulement : une carte core de 2 comps ne compte pas double.
        best = max(((CORE_POINTS * w, f"core {comp.name}") if card in comp.core else
                    (ADDON_POINTS * w, f"add-on {comp.name}") if card in comp.addons else (0, "")
                    for comp, w in targets), default=(0, ""))
        if best[0]:
            value += best[0]
            reasons.append(best[1])
        if copies[card.id] >= 2:
            value += TRIPLE_POINTS
            reasons.append("triple")
        elif copies[card.id] == 1:
            value += PAIR_POINTS
            reasons.append("pair")
        if value:
            picks.append(ShopPick(card, value, reasons))
    picks.sort(key=lambda p: p.value, reverse=True)
    return picks
