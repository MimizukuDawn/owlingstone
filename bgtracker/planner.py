"""Plan du tour (recrutement) : monter de tier ou pas, quoi acheter, relancer ou geler, en tenant compte
de ma vie, de la force de mon plateau et du plateau attendu de mon prochain adversaire.

Qui fait quoi :
- expected_board(...) : le plateau probable de mon prochain adversaire MAINTENANT. Son dernier plateau vu
  date souvent de 4 à 7 tours, et les plateaux grossissent très vite (×1,6 par tour en moyenne) : on le
  "fait vieillir" avec la courbe de croissance apprise sur mes parties (voir GROWTH). Jamais affronté =
  plateau typique du lobby à ce tour.
- fight_plan(game) : appelé dans le fil qui lit le log (comme combat.combat_info). Simule avec combat.py
  mon plateau (+ les serviteurs de ma main, posés gratuitement) contre ce plateau attendu : tel quel,
  avec chaque serviteur de la boutique en plus, avec tout mon or dépensé en achats, et en montant de tier
  avec l'or qui reste. Dégâts plafonnés par BACON_COMBAT_DAMAGE_CAP comme dans le jeu.
- turn_plan(...) : appelé par la fenêtre. Mélange ces simulations avec les points "comp" de advice.py
  (core / add-on / triple) et l'économie / l'avantage de cartes (data/economy.yaml) et sort des conseils lisibles : Level ou pas, Buy, Sell, Roll, Freeze, et
  le danger pour ma vie.

Données : la courbe de croissance vient de mes 13 parties Solo du patch actuel (26-30/10/2026, 150 combats,
voir NOTES.md). Les statistiques par tour de Firestone / HSReplay ne sont pas accessibles (API bloquée,
pages rendues côté navigateur). La courbe "quand monter" suit la courbe standard des guides
(T2 au tour 2, T3 tour 5, T4 tour 7, T5 tour 9, T6 tour 12), décalée sur le rythme réel des lobbies.
"""
import math
import re
from pathlib import Path

import yaml

from .cards import get_db
from .combat import _odds, snapshot_minion
from .paths import DATA_DIR

ECONOMY_PATH = DATA_DIR / "economy.yaml"
HERO_CURVES_PATH = DATA_DIR / "hero_curves.yaml"

MAX_BOARD = 7

# log(attaque + vie totales du plateau adverse) = a + b × tour + c × tier de l'adversaire.
# Ajusté sur 150 combats (13 parties, patch 36.6.3) : écart type résiduel 0,73 (≈ ×2).
GROWTH = (0.71, 0.496, 0.154)
GROWTH_SD = 0.73
# Un adversaire plus fort (ou plus faible) que la moyenne le reste en partie : on garde 90 % de son
# écart à la courbe par tranche de 2 tours. Sur 57 paires (même adversaire revu plus tard), erreur
# 0,80 en log contre 3,25 en reprenant tel quel le vieux plateau (×2 contre ×25).
PERSISTENCE = 0.9
# Un vieux plateau "grossi" reste trop faible : l'adversaire a aussi gagné des effets, des boucliers, des
# dorés... que la simple multiplication des stats ne donne pas. +0,5 en log (×1,65) colle le mieux aux
# résultats réels (59 combats : Brier 0,55 contre 0,66 sans, 1,36 avec le vieux plateau tel quel).
SEEN_BOOST = 0.5
# Nombre moyen de serviteurs du plateau adverse selon le tour (mêmes 150 combats)
BOARD_COUNT = {1: 1, 2: 1, 3: 2, 4: 3, 5: 4, 6: 5, 7: 6, 8: 7}

# Tier visé à la FIN de chaque tour (courbe standard, tempo moyen des lobbies observé dans mes logs)
LEVEL_CURVE = {1: 1, 2: 2, 3: 2, 4: 2, 5: 3, 6: 3, 7: 4, 8: 4, 9: 5, 10: 5, 11: 5, 12: 6}
# Exemplaires de chaque serviteur dans le pool selon son tier, et taille de la boutique selon mon tier
# (valeurs connues du mode, à revérifier si Blizzard les change)
POOL_COPIES = {1: 16, 2: 15, 3: 13, 4: 11, 5: 9, 6: 7, 7: 5}
SHOP_SLOTS = {1: 3, 2: 4, 3: 4, 4: 5, 5: 5, 6: 6, 7: 6}
MINION_COST = 3
FIGHT_WEIGHT = 5            # points pour un combat qui passe de perdu à gagné (une carte core de comp = 3)

_yaml_cache = {}


def _load_yaml(path, build):
    """build(contenu du fichier YAML), gardé tant que le fichier ne change pas ({} s'il n'existe pas)."""
    path = Path(path)
    if not path.exists():
        return {}
    mtime = path.stat().st_mtime
    cached = _yaml_cache.get(path)
    if cached and cached[0] == mtime:
        return cached[1]
    with open(path, encoding="utf-8") as f:
        table = build(yaml.safe_load(f) or {})
    _yaml_cache[path] = (mtime, table)
    return table


def load_economy(path=ECONOMY_PATH):
    """{id de carte: {"value", "kind", "when", "note"}} depuis data/economy.yaml (relu s'il a changé)."""
    def build(raw):
        by_name = {c.name: c for c in get_db().pool_minions(duos=True)}
        return {by_name[name].id: dict(info, name=name) for name, info in raw.items() if name in by_name}
    return _load_yaml(path, build)


def load_hero_curves(path=HERO_CURVES_PATH):
    """{nom du héros ou du pouvoir: réglages} depuis data/hero_curves.yaml (voir l'en-tête du fichier)."""
    return _load_yaml(path, dict)


def hero_profile(hero, powers=(), turn=1, table=None):
    """Réglages du plan pour mon héros : courbe de tier, or à garder pour le pouvoir, préférences.

    powers : [(nom, card_id, prix actuel, déjà utilisé ce tour)] de mes pouvoirs héroïques en jeu.
    Héros absent de hero_curves.yaml : courbe standard, et un pouvoir à 1-2 d'or est gardé dès le tour 3."""
    table = load_hero_curves() if table is None else table
    entry = table.get(hero) or next((table[p[0]] for p in powers if p[0] in table), None) or {}
    curve, tier = {}, 1
    marks = entry.get("curve") or LEVEL_CURVE
    for t in range(1, 21):
        tier = marks.get(t, tier)
        curve[t] = tier
    usable = [p for p in powers if p[2] and p[2] > 0 and not p[3]]
    rule = entry.get("power")
    reserve, power_name = 0, None
    if usable and rule != "never":
        name, _, cost, _ = usable[0]
        if isinstance(rule, dict):
            use = rule.get("from_turn", 1) <= turn <= rule.get("until_turn", 99)
        else:
            use = cost <= 2 and turn >= 3          # règle automatique
        if use:
            reserve, power_name = cost, name
    return {"curve": curve, "custom_curve": bool(entry.get("curve")), "reserve": reserve, "power": power_name,
            "level_bias": entry.get("level_bias", 0), "minion_cost": entry.get("minion_cost", MINION_COST),
            "buy_bonus": entry.get("buy_bonus") or {}, "pair_bonus": entry.get("pair_bonus", 0),
            "note": entry.get("note", ""), "hero": hero}


def economy_value(card_id, turn, economy=None):
    """(points, raison) d'une carte d'économie / d'avantage de cartes, ou (0, ""). Plus tôt = plus de tours
    pour en profiter : ×1,2 jusqu'au tour 4, ×1 au tour 6, puis moins, ×0,3 au plus bas."""
    info = (load_economy() if economy is None else economy).get(card_id)
    if not info:
        return 0.0, ""
    factor = max(0.3, min(1.2, (14 - turn) / 8))
    word = "economy" if info.get("kind") == "gold" else "cards"
    return info.get("value", 0) * factor, f"{word}: {info.get('note', '')}".strip()


PLAN_RUNS, PLAN_SECONDS = 500, 0.08          # par plateau testé (une dizaine par boutique)


# ---------------------------------------------------------------- synergie avec mon plateau actuel
RACE_WORDS = {"BEAST": "Beast", "MECHANICAL": "Mech", "MURLOC": "Murloc", "DEMON": "Demon", "DRAGON": "Dragon",
              "ELEMENTAL": "Elemental", "PIRATE": "Pirate", "QUILBOAR": "Quilboar", "UNDEAD": "Undead",
              "NAGA": "Naga", "ABERRATION": "Aberration"}
# Mécaniques : (ce qui en "a" / en produit, ce qui en profite). Une carte qui profite d'une mécanique que
# mon plateau a (ou l'inverse) = synergie.
MECHANICS = {
    "Deathrattle": (r"^Deathrattle|\bDeathrattle:", r"(your|friendly|a|another)\b[^.]*\bDeathrattle|Deathrattles"),
    "Battlecry": (r"^Battlecry|\bBattlecry:", r"(your|friendly|a)\b[^.]*\bBattlecr(y|ies)\b(?!:)|Battlecries"),
    "Divine Shield": (r"Divine Shield", r"(loses?|lost) Divine Shield|with Divine Shield"),
    "Tavern spell": (r"(Get|get|Discover)[^.]*(Tavern spell|spell)", r"(cast|buy)[^.]*Tavern spell|Tavern spells give"),
    "Blood Gem": (r"(Get|get|plays?)[^.]*Blood Gem", r"Blood Gems (give|played)|Whenever a Blood Gem"),
    "Magnetic": (r"Magnetic|Volumizer", r"Magnetize|Magnetic Mech"),
    "Golden": (r"Make[^.]*Golden|Golden minion", r"for each Golden|Golden minions? you"),
}
SYNERGY_POINT, SYNERGY_MAX = 0.6, 2.5


def _mentions_race(text, race):
    """Le texte profite-t-il de cette race ? ("your Beasts", "after you play a Murloc"). Donner ou invoquer
    une race ("Get a random Beast", "Summon a 1/1 Murloc") n'est pas en profiter."""
    word = RACE_WORDS.get(race)
    if not word:
        return False
    text = re.sub(r"\b(Get|get|Discover|discover|Summon|summon)\b[^.]*", "", text)
    return bool(re.search(rf"\b{word}s?\b", text))


def board_synergy(card, board):
    """(points, raison) : à quel point cette carte de la boutique va avec mon plateau (et ma main) MAINTENANT,
    même si elle ne sert à aucune comp plus tard. Ex. un Beast quand j'ai des cartes "your Beasts", une carte
    "after you cast a Tavern spell" quand mon plateau en donne."""
    text = card.text or ""
    races = [r for r in card.races if r != "ALL"] or (list(RACE_WORDS) if "ALL" in card.races else [])
    points, why = 0.0, []
    for b in board:
        if b is None or b.id == card.id:
            continue
        btext = b.text or ""
        b_races = [r for r in b.races if r != "ALL"] or (list(RACE_WORDS) if "ALL" in b.races else [])
        hit = None
        if any(_mentions_race(btext, r) for r in races):
            hit = b.name                                      # mon serviteur profite de sa race
        elif any(_mentions_race(text, r) for r in b_races):
            hit = b.name                                      # elle profite de la race de mon serviteur
        else:
            for name, (has, cares) in MECHANICS.items():
                if ((re.search(has, btext) and re.search(cares, text))
                        or (re.search(has, text) and re.search(cares, btext))):
                    hit = f"{b.name} ({name})"
                    break
        if hit:
            points += SYNERGY_POINT
            why.append(hit)
    points = min(points, SYNERGY_MAX)
    return points, ("synergy now: " + ", ".join(why[:2]) + (f" +{len(why) - 2}" if len(why) > 2 else "")) if why else ""


# --------------------------------------------------------------------------- plateau attendu
def board_power(board):
    return sum(m["attack"] + m["health"] for m in board)


def typical_power(turn, tier):
    """Attaque + vie totales d'un plateau typique de ce tour à ce tier (médiane des lobbies)."""
    a, b, c = GROWTH
    return math.exp(a + b * turn + c * (tier or 1))


def strength_percentile(power, turn, tier):
    """Où se situe ce plateau par rapport aux plateaux du lobby au même tour (0-100)."""
    if power <= 0:
        return 0
    z = (math.log(power) - math.log(typical_power(turn, tier))) / GROWTH_SD
    return 50 * (1 + math.erf(z / math.sqrt(2)))


def _filler(attack, health, tier):
    """Serviteur sans effet (inconnu) : seulement des stats."""
    return {"card_id": "", "attack": attack, "health": health, "max_health": health, "golden": False,
            "tier": tier, "races": [], "windfury": 0, "data": (0, 0), "taunt": False, "ds": False,
            "venom": False, "poison": False, "reborn": False, "stealth": False, "cleave": False}


def expected_board(turn, tier, last=None):
    """(plateau probable de l'adversaire à ce tour, explication).

    last = son dernier combat contre moi (game.fights) ou None. Ses serviteurs gardent leurs effets ;
    leurs stats sont multipliées pour coller à la force attendue maintenant, et des serviteurs sans effet
    complètent le plateau s'il en avait moins que d'habitude à ce tour."""
    tier = tier or 1
    target = math.log(typical_power(turn, tier))
    seen = list(last["board"]) if last and last.get("board") else []
    if seen and last["turn"] >= 4:
        then = math.log(max(board_power(seen), 1)) - math.log(typical_power(last["turn"], last.get("opp_tier", 1)))
        target += PERSISTENCE ** ((turn - last["turn"]) / 2) * then + SEEN_BOOST
    power = math.exp(target)
    fresh = bool(seen) and last["turn"] == turn      # vu ce tour-ci (pendant un combat) : rien à vieillir
    count = len(seen) if fresh else max(len(seen), BOARD_COUNT.get(turn, MAX_BOARD))
    filler_tier = max(1, tier - 1)
    board = [dict(m) for m in seen]
    while len(board) < count:
        board.append(_filler(1, 1, filler_tier))
    if fresh:
        k = 1.0
    else:
        k = power / max(board_power(board), 1)
    for m in board:
        m["attack"] = max(0, round(m["attack"] * k))
        m["health"] = max(1, round(m["health"] * k))
        m["max_health"] = m["health"]
    if not seen:
        basis = "typical board for this turn (never fought)"
    elif fresh:
        basis = "their board this turn"
    else:
        basis = f"their turn {last['turn']} board grown ×{k:.1f}"
    return board, basis


# --------------------------------------------------------------------------------- simulations
def board_value(gain, turn, tier):
    """Valeur pour la suite de la partie des stats ajoutées à mon plateau (elles restent après ce combat) :
    3 points pour un plateau typique de ce tour en plus, au plus 2. Ex. un 3/3 au tour 3 ≈ 1 point."""
    return max(-2.0, min(2.0, 3 * gain / typical_power(turn, tier)))


def fight_value(sim, health):
    """Valeur d'un combat simulé (plus haut = mieux) : gagner, faire mal, et surtout ne pas perdre de vie
    quand il en reste peu (les dégâts reçus pèsent 1/vie : 5 dégâts à 30 PV ≈ 0,17, à 8 PV ≈ 0,6)."""
    h = max(health or 30, 3)
    return (sim["win"] / 100 + 0.02 * sim["avg_dealt"] - sim["avg_taken"] / h
            - 3 * sim["lethal_me"] / 100)


def add_minion(team, m):
    """Mon plateau avec ce serviteur en plus ; plateau plein : il remplace le plus faible (s'il est mieux)."""
    if len(team) < MAX_BOARD:
        return team + [m]
    weakest = min(range(len(team)), key=lambda i: team[i]["attack"] + team[i]["health"])
    if team[weakest]["attack"] + team[weakest]["health"] >= m["attack"] + m["health"]:
        return list(team)
    return team[:weakest] + team[weakest + 1:] + [m]


def fight_plan(game, runs=PLAN_RUNS, seconds=PLAN_SECONDS):
    """Simulations du tour pour turn_plan (None hors recrutement). Gardées en cache tant que rien ne change."""
    if (game is None or not game.is_battlegrounds or game.is_duos or game.is_over or game.in_combat
            or game.my_hero is None or game.hero_options):
        return None
    shop = game.shop
    opp = game.next_opponent_id
    if opp is None or not shop:
        return None
    turn, tier, gold, health = game.turn, game.tavern_tier or 1, game.my_gold or 0, game.my_health
    opp_hero = game.hero_of(opp)
    opp_tier = (opp_hero.get("PLAYER_TECH_LEVEL") if opp_hero else None) or 1
    last = game.last_fight_against(opp)
    if last is not None and "my_board" not in last:
        last = None
    team = [snapshot_minion(e) for e in game.my_board]
    team += [snapshot_minion(e) for e in game.my_hand if e.is_minion][:MAX_BOARD - len(team)]
    hero = game.my_hero.name
    powers = tuple((e.name, e.card_id, e.get("COST", 0), bool(e.get("EXHAUSTED"))) for e in game.my_hero_powers)
    profile = hero_profile(hero, powers, turn)
    budget = max(0, gold - profile["reserve"])         # l'or du pouvoir héroïque est mis de côté
    offers = [(get_db().base(e.card_id), snapshot_minion(e), e.get("COST") or profile["minion_cost"])
              for e in shop if e.card_id]
    offers = [o for o in offers if o[0] is not None]
    up, roll = game.upgrade_cost, game.reroll_cost
    ge = game.game_entity
    cap = ge.get("BACON_COMBAT_DAMAGE_CAP") if ge and ge.get("BACON_COMBAT_DAMAGE_CAP_ENABLED") else None
    key = (repr(team), repr([o[1] for o in offers]), gold, up, opp, opp_tier, turn, cap, health,
           last["turn"] if last else None, hero, powers)
    cache = getattr(game, "_plan_cache", None)
    if cache is not None and cache[0] == key:
        return cache[1]

    theirs, basis = expected_board(turn, opp_tier, last)
    bonuses = game.my_bonuses
    counters = (game.player_counters(game.local_player_id), (last or {}).get("counters", ({}, {}))[1])
    deity = (game.deity_seen.get(game.my_player_id), game.deity_seen.get(opp))

    def sim(mine, my_tier=tier):
        if not mine:
            mine = []
        return _odds(game, mine, theirs, my_tier, opp_tier, opp, None, cap, deity, bonuses["gem"],
                     bonuses["spell"], runs, seconds, counters)

    base = sim(team)
    per_card, gains = {}, {}
    for card, m, cost in offers:
        if card.id not in per_card:
            board = add_minion(team, m)
            per_card[card.id] = sim(board)
            gains[card.id] = board_power(board) - board_power(team)

    def best_buys(money):
        """Achats gloutons (meilleure valeur de combat d'abord) avec cet or."""
        ranked = sorted(offers, key=lambda o: fight_value(per_card[o[0].id], health), reverse=True)
        chosen, board = [], list(team)
        for card, m, cost in ranked:
            if cost <= money and fight_value(per_card[card.id], health) > fight_value(base, health):
                chosen.append(card.id)
                board = add_minion(board, m)
                money -= cost
        return chosen, board

    spend_ids, spend_board = best_buys(budget)
    spend = (base if not spend_ids else per_card[spend_ids[0]] if len(spend_ids) == 1 else sim(spend_board))
    level = None
    if up is not None and budget >= up:
        lvl_ids, lvl_board = best_buys(budget - up)
        level = {"buys": lvl_ids, "sim": sim(lvl_board) if lvl_ids else base}
    plan = {
        "turn": turn, "tier": tier, "gold": gold, "health": health, "cap": cap,
        "upgrade_cost": up, "reroll_cost": roll,
        "opponent": opp_hero.name if opp_hero else None, "opp_tier": opp_tier,
        "opp_health": game.player_health(opp), "basis": basis,
        "opp_power": board_power(theirs), "my_power": board_power(team),
        "strength": strength_percentile(board_power(team), turn, tier),
        "base": base, "per_card": per_card, "power_gain": gains, "spend": {"buys": spend_ids, "sim": spend}, "level": level,
        "team_size": len(team), "board_size": len(game.my_board),
        "weakest": _by_weakness(game.my_board),
        "profile": profile, "budget": budget,
        "team_ids": [get_db().base(e.card_id).id for e in game.my_board + game.my_hand
                     if e.card_id and get_db().base(e.card_id) and e.is_minion],
    }
    game._plan_cache = (key, plan)
    return plan


def _by_weakness(board):
    """[(id de base, nom, attaque + vie, doré)] de mon plateau, du plus faible au plus fort ; dorés en dernier."""
    db = get_db()
    ranked = sorted(board, key=lambda e: (e.is_golden, e.get("ATK", 0) + e.get("HEALTH", 0) - e.get("DAMAGE", 0)))
    return [(db.base(e.card_id).id if db.base(e.card_id) else e.card_id, e.name,
             e.get("ATK", 0) + e.get("HEALTH", 0) - e.get("DAMAGE", 0), e.is_golden) for e in ranked]


def sell_choice(board, protect=(), turn=6, tier=3, economy=None):
    """(nom, raison) du serviteur à vendre pour faire de la place, ou None.

    On garde : les cartes de ma comp et les paires (protect), les dorés, les cartes d'économie qui
    rapportent tant qu'elles sont sur le plateau. On vend d'abord : les cartes qui rapportent en les
    vendant (Patient Scout...), celles dont le Battlecry d'économie est déjà joué, puis le plus faible."""
    economy = load_economy() if economy is None else economy
    protect = set(protect)
    best = None
    for cid, name, power, golden in board:
        if cid in protect or golden:
            continue
        info = economy.get(cid) or {}
        keep = board_value(power, turn, tier)
        why = "weakest"
        if info.get("when") == "board":
            keep += economy_value(cid, turn, economy)[0]
        elif info.get("when") == "sell":
            keep -= 1.5
            why = info.get("note", "reward when sold")
        elif info.get("when") == "play":
            keep -= 0.5
            why = "its Battlecry is already used"
        if best is None or keep < best[0]:
            best = (keep, name, why)
    return (best[1], best[2]) if best else None


# ------------------------------------------------------------------------------ chances de relance
def find_chance(wanted, tier, available):
    """Chance de voir au moins une des cartes voulues dans la prochaine boutique.

    wanted : Cards cherchées ; available : toutes les Cards du pool de ce lobby jusqu'à mon tier.
    Chaque case de la boutique tire un serviteur au hasard parmi les exemplaires restants (on suppose
    le pool plein)."""
    total = sum(POOL_COPIES.get(c.tier, 7) for c in available)
    ids = {c.id for c in available}
    hit = sum(POOL_COPIES.get(c.tier, 7) for c in {c.id: c for c in wanted}.values() if c.id in ids)
    if not total or not hit:
        return 0.0
    return 1 - (1 - hit / total) ** SHOP_SLOTS.get(tier, 5)


# ----------------------------------------------------------------------------------- conseils
def danger_level(plan):
    """(mot, couleur, phrase) : à quel point le prochain combat menace ma vie."""
    s, hp = plan["base"], plan["health"] or 0
    if s["lethal_me"] >= 10:
        return "critical", "#e57373", f"{s['lethal_me']:.0f}% chance the next fight kills you"
    if hp and s["loss"] >= 50 and s["taken"] >= hp / 3:
        return "high", "#e0a050", f"likely loss for ~{s['taken']:.0f} of your {hp} health"
    if s["loss"] >= 50 and s["taken"] >= hp / 6:
        return "medium", "#d9c060", f"likely loss (~{s['taken']:.0f} damage)"
    if s["loss"] >= 50:
        return "low", "#7fe07f", f"likely loss, but only ~{s['taken']:.0f} damage"
    return "low", "#7fe07f", "you should win or trade"


def level_advice(plan, foregone=0.0):
    """(monter ?, raison) ou (None, raison) si on ne peut pas monter ce tour."""
    up, tier, turn = plan["upgrade_cost"], plan["tier"], plan["turn"]
    profile = plan.get("profile") or hero_profile(None, (), turn, {})
    gold = plan.get("budget", plan["gold"])
    mc = profile["minion_cost"]
    if up is None:
        return None, "max tier"
    if plan["level"] is None:
        after = f" after your hero power" if profile["reserve"] else ""
        return None, f"level costs {up}, you have {gold}{after}"
    hp = max(plan["health"] or 30, 3)
    lvl, spend = plan["level"]["sim"], plan["spend"]["sim"]
    curve = profile["curve"].get(turn, 6)
    behind = curve - (tier + 1)                 # >= 0 : monter me garde sur la courbe
    benefit = (2.0 if behind == 0 else 3.0 if behind > 0 else 0.7) + profile["level_bias"]
    if up <= 2:
        benefit += 1.0                          # presque gratuit
    if gold - up >= mc:
        benefit += 0.5                          # je peux encore acheter
    extra = max(0.0, lvl["avg_taken"] - spend["avg_taken"])
    risk = (extra * 10 / max(hp - 5, 1) + max(0.0, lvl["lethal_me"] - spend["lethal_me"]) / 5
            + foregone)                    # achats ratés (cartes de comp, stats, économie)
    if lvl["lethal_me"] >= 15 and lvl["lethal_me"] > spend["lethal_me"] + 5:
        return False, (f"leveling leaves you {lvl['lethal_me']:.0f}% to die "
                       f"(buying: {spend['lethal_me']:.0f}%): buy instead")
    if hp <= 15 and tier >= 4 and behind <= 0 and plan["base"]["loss"] >= 50:
        return False, f"only {plan['health']} health and you're losing: spend on your board now"
    where = ("on curve" if behind == 0 else f"{behind} tier behind curve" if behind > 0
             else "ahead of curve")
    if profile["custom_curve"]:
        where = where.replace("curve", f"{profile['hero']}'s curve")
    costs = []
    if extra >= 0.5:
        costs.append(f"costs ~{extra:.1f} health this fight")
    if foregone >= 0.3:
        costs.append("skips good buys")
    if benefit > risk:
        return True, f"Tier {tier + 1} for {up} gold: {where}" + (f" ({', '.join(costs)})" if costs else "")
    return False, f"wait ({where}): leveling " + (" and ".join(costs) if costs else "isn't worth it yet")


def turn_plan(plan, comp_picks=(), wanted=(), available=(), protect=()):
    """Conseils lisibles du tour.

    plan : résultat de fight_plan ; comp_picks : advice.shop_advice (points comp / triple de chaque carte) ;
    wanted : Cards que je cherche (core manquants de ma comp, 3e exemplaire d'une paire) ;
    available : Cards du pool de ce lobby jusqu'à mon tier (pour les chances de relance) ;
    protect : ids des cartes à ne pas vendre (cartes de ma comp, paires pour un triple).
    Renvoie un dict : level, buy [(Card, gain de victoire en points de %, valeur, raisons, valeur durable)], sell, roll, freeze,
    danger, fight (le combat attendu)."""
    db = get_db()
    hp = plan["health"]
    profile = plan.get("profile") or hero_profile(None, (), plan["turn"], {})
    gold = plan.get("budget", plan["gold"])            # sans l'or gardé pour le pouvoir héroïque
    mc = profile["minion_cost"]
    base = plan["base"]
    comp_value = {p.card.id: p for p in comp_picks}
    economy = load_economy()
    team = [db.get(i) for i in plan.get("team_ids", ())]
    # Valeur de chaque carte : points comp (0-6) + stats + économie + gain au prochain combat (+20 % de victoire = 1 point)
    buys = []
    for cid, s in plan["per_card"].items():
        card = db.get(cid)
        gain = s["win"] - base["win"]
        saved = base["avg_taken"] - s["avg_taken"]
        pick = comp_value.get(cid)
        stats = board_value(plan.get("power_gain", {}).get(cid, 0), plan["turn"], plan["tier"])
        econ, econ_why = economy_value(cid, plan["turn"], economy)
        syn, syn_why = board_synergy(card, team)
        lasting = (pick.value if pick else 0) + stats + econ + syn  # ce qui reste après ce combat
        value = lasting + FIGHT_WEIGHT * (fight_value(s, hp) - fight_value(base, hp))
        reasons = list(pick.reasons) if pick else []
        hero_bonus = max((v for race, v in profile["buy_bonus"].items() if race in card.races), default=0)
        if profile["pair_bonus"] and "pair" in reasons:
            hero_bonus += profile["pair_bonus"]
        if hero_bonus:
            lasting += hero_bonus
            value += hero_bonus
            reasons.append(f"{profile['hero']}: {profile['note'].split(':')[0]}")
        if econ_why:
            reasons.append(econ_why)
        if syn_why:
            reasons.append(syn_why)
        if stats >= 0.5 and gain < 1:
            reasons.append("stats for later fights")
        fight = ([f"+{gain:.0f}% win"] if gain >= 1 else []) + ([f"-{saved:.1f} dmg taken"] if saved >= 0.5 else [])
        if fight:
            reasons.append(", ".join(fight))
        if value > 0.3:
            buys.append((card, gain, value, reasons, lasting))
    buys.sort(key=lambda b: b[2], reverse=True)

    # Ce que monter de tier me fait rater : la valeur (comp, combat, stats, économie) des achats que l'or
    # restant ne paie plus
    values = sorted((b[4] for b in buys), reverse=True)      # (les dégâts de ce combat : dans level_advice)
    up = plan["upgrade_cost"] or 0
    foregone = 0.6 * (sum(values[:gold // mc]) - sum(values[:max(0, gold - up) // mc]))
    want_level, level_why = level_advice(plan, foregone)
    money = gold - (plan["upgrade_cost"] if want_level else 0)
    affordable = max(0, money) // mc
    shown = buys[:max(affordable, 1) + 1]
    plan_buys = buys[:affordable]
    sell = None
    if plan_buys and plan["team_size"] + len(plan_buys) > MAX_BOARD:
        choice = sell_choice(plan.get("weakest", ()), protect, plan["turn"], plan["tier"], economy)
        if choice:
            sell = f"{choice[0]} ({choice[1]})"
            money += 1                                   # vendre rapporte 1
    left = money - mc * len(plan_buys)
    # Relance : l'or non dépensé est perdu à la fin du tour
    roll_cost = plan["reroll_cost"]
    chance = find_chance(wanted, plan["tier"], available) if wanted else 0.0
    roll = None
    if roll_cost == 0:
        roll = "Free roll: always use it"
    elif plan_buys and left >= roll_cost:
        n = left // roll_cost
        roll = f"Then roll {n}× with your last {left} gold (unspent gold is lost)"
    elif not plan_buys and money >= roll_cost:
        # Rien qui vaille le coup ici : relancer, en gardant 3 d'or par serviteur à acheter si je trouve
        n = max(0, money - mc) // roll_cost
        roll = ("Nothing here is worth it: roll" + (f" up to {n}× and keep {mc} gold to buy what you find"
                                                     if n else f" {money // roll_cost}×"))
    roll_pro = f"{chance * 100:.0f}% per roll to see a card you want" if chance else ""
    freeze = None
    keep = [b for b in buys[affordable:] if b[2] >= 3]
    if keep and left < mc:
        freeze = f"Freeze to keep {keep[0][0].name} for next turn"
    danger = danger_level(plan)
    power = (f"Use {profile['power']} ({profile['reserve']} gold, kept aside above)" if profile["reserve"] else None)
    return {"level": want_level, "level_why": level_why, "buy": shown[:3], "plan_buys": [b[0] for b in plan_buys],
            "sell": sell, "roll": roll, "roll_pro": roll_pro, "freeze": freeze, "danger": danger,
            "power": power, "hero_note": profile["note"]}
