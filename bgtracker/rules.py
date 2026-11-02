"""Règles de préférence (phase 4) : lire data/rules.yaml et juger chaque comp pour la partie en cours.

Qui fait quoi :
- load_rules() lit le fichier YAML (modifiable à la main) et vérifie chaque règle (comp connue,
  un seul effet, conditions connues) pour qu'une faute de frappe se voie tout de suite.
- check(cond, ctx) dit si une condition est vraie, fausse, ou encore inconnue (None) : avant que
  les races du lobby soient sûres, une condition sur les races ne peut pas encore être jugée.
- judge(comp_id, rules, ctx) applique toutes les règles d'une comp : cachée ou non, bonus, notes.

Effets possibles (un par règle) :
  only_if    la comp n'est proposée que si la condition est vraie
  never_if   la comp est cachée si la condition est vraie
  prefer_if  +bonus points quand la condition est vraie (bonus par défaut : 3, soit une carte core)
  avoid_if   -bonus points quand la condition est vraie
Une condition inconnue ne fait rien : on ne cache pas une comp tant qu'on n'est pas sûr.
"""
from dataclasses import dataclass, field
from pathlib import Path

import yaml

from .paths import DATA_DIR

RULES_PATH = DATA_DIR / "rules.yaml"

EFFECTS = ("only_if", "never_if", "prefer_if", "avoid_if")
CONDITIONS = ("lobby_has", "lobby_has_any", "lobby_lacks", "deity", "hero", "hero_power")
DEFAULT_BONUS = 3


@dataclass
class Rule:
    comps: list           # ids de comps visées
    effect: str           # un de EFFECTS
    cond: dict            # {condition: valeur}, toutes doivent être vraies ({} = toujours)
    bonus: float = DEFAULT_BONUS
    note: str = ""


@dataclass
class Verdict:
    hidden: bool = False
    bonus: float = 0
    notes: list = field(default_factory=list)


def load_rules(path=RULES_PATH, comp_ids=None):
    """Lit rules.yaml (absent = aucune règle). Lève une erreur claire si une règle est mal écrite."""
    path = Path(path)
    if not path.exists():
        return []
    with open(path, encoding="utf-8") as f:
        raw = yaml.safe_load(f) or []
    rules, errors = [], []
    for i, r in enumerate(raw, 1):
        comps = r.get("comp")
        comps = [comps] if isinstance(comps, str) else list(comps or [])
        effects = [e for e in EFFECTS if e in r]
        where = f"règle {i}"
        if not comps:
            errors.append(f"{where} : 'comp' manquant")
        if comp_ids is not None:
            errors += [f"{where} : comp inconnue '{c}'" for c in comps if c not in comp_ids]
        if len(effects) != 1:
            errors.append(f"{where} : il faut un seul effet parmi {', '.join(EFFECTS)}")
            continue
        cond = r[effects[0]] or {}
        errors += [f"{where} : condition inconnue '{k}'" for k in cond if k not in CONDITIONS]
        rules.append(Rule(comps=comps, effect=effects[0], cond=cond,
                          bonus=r.get("bonus", DEFAULT_BONUS), note=r.get("note", "")))
    if errors:
        raise ValueError("Erreurs dans rules.yaml : " + "; ".join(errors))
    return rules


def _as_list(v):
    return [v] if isinstance(v, str) else list(v)


def check(cond, ctx):
    """True / False, ou None si une info manque encore (ctx = races, deity, hero ; None = inconnu)."""
    unknown = False
    for key, want in cond.items():
        if key == "hero_power":                 # un de mes pouvoirs héroïques actuels (il peut changer)
            have = ctx.get("hero_power")
            if have is None:
                unknown = True
            elif not {str(w).lower() for w in _as_list(want)} & {h.lower() for h in have}:
                return False
            continue
        if key in ("deity", "hero"):
            have = ctx.get(key)
            if have is None:
                unknown = True
            elif have.lower() != str(want).lower():
                return False
            continue
        races = ctx.get("races")
        if races is None:
            unknown = True
            continue
        want = _as_list(want)
        ok = {"lobby_has": all(r in races for r in want),
              "lobby_has_any": any(r in races for r in want),
              "lobby_lacks": not any(r in races for r in want)}[key]
        if not ok:
            return False
    return None if unknown else True


def judge(comp_id, rules, ctx):
    """Résultat de toutes les règles qui visent cette comp."""
    v = Verdict()
    for rule in rules:
        if comp_id not in rule.comps:
            continue
        result = check(rule.cond, ctx)
        if (rule.effect == "only_if" and result is False) or (rule.effect == "never_if" and result is True):
            v.hidden = True
        elif result is True and rule.effect in ("prefer_if", "avoid_if"):
            v.bonus += rule.bonus if rule.effect == "prefer_if" else -rule.bonus
        else:
            continue
        if rule.note:
            v.notes.append(rule.note)
    return v
