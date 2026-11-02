"""Tests du plan du tour (planner.py) : plateau attendu de l'adversaire, monter ou pas, achats, relances."""
import pytest

from bgtracker.advice import ShopPick
from bgtracker.cards import get_db
from bgtracker.planner import (board_power, economy_value, expected_board, find_chance, level_advice,
                               load_economy, sell_choice, strength_percentile, turn_plan, typical_power)
from bgtracker.powerlog import PowerLogParser
from tests.test_comps import _cards
from tests.test_powerlog import GAME1


def m(card_id="", attack=1, health=1, **kw):
    return dict(card_id=card_id, attack=attack, health=health, tier=kw.pop("tier", 1), **kw)


def sim(win=50, loss=50, taken=5.0, lethal=0, dealt=5.0):
    return {"win": win, "tie": 100 - win - loss, "loss": loss, "taken": taken, "dealt": dealt,
            "avg_taken": taken * loss / 100, "avg_dealt": dealt * win / 100, "lethal_me": lethal, "lethal": 0}


def plan(**kw):
    p = {"turn": 6, "tier": 3, "gold": 8, "health": 30, "cap": 10, "upgrade_cost": 7, "reroll_cost": 1,
         "base": sim(), "per_card": {}, "power_gain": {}, "spend": {"buys": [], "sim": sim()},
         "level": {"buys": [], "sim": sim()}, "team_size": 4, "weakest": []}
    p.update(kw)
    return p


def test_old_board_grows_with_the_turns():
    """Un plateau vu au tour 5 est bien plus gros au tour 9 ; il garde ses cartes (et leurs effets)."""
    last = {"turn": 5, "opp_tier": 3, "board": [m("BG31_803", 3, 3), m(attack=4, health=5)]}
    board, basis = expected_board(9, 4, last)
    assert board[0]["card_id"] == "BG31_803"
    assert len(board) == 7                                 # complété : 7 serviteurs d'habitude au tour 9
    assert board_power(board) > 10 * board_power(last["board"])
    assert "turn 5" in basis
    # Vu ce tour-ci : rien ne change
    now = {"turn": 9, "opp_tier": 4, "board": last["board"]}
    assert board_power(expected_board(9, 4, now)[0]) == board_power(last["board"])


def test_unknown_opponent_gets_a_typical_board():
    board, basis = expected_board(8, 4, None)
    assert "typical" in basis
    assert abs(board_power(board) - typical_power(8, 4)) < 0.1 * typical_power(8, 4)
    assert 49 < strength_percentile(typical_power(8, 4), 8, 4) < 51


def test_find_chance_grows_with_wanted_cards():
    db = get_db()
    pool = [c for c in db.pool_minions() if c.tier and c.tier <= 2][:40]
    one = find_chance(pool[:1], 2, pool)
    assert 0 < one < find_chance(pool[:3], 2, pool) < 1
    assert find_chance([], 2, pool) == 0


def test_level_on_curve_when_it_costs_nothing():
    ok, why = level_advice(plan(turn=7, tier=3, gold=8, upgrade_cost=7))
    assert ok is True and "Tier 4" in why


def test_dont_level_into_lethal():
    p = plan(health=6, level={"buys": [], "sim": sim(win=0, loss=100, taken=8, lethal=80)},
             spend={"buys": ["x"], "sim": sim(win=60, loss=30, taken=4, lethal=5)})
    ok, why = level_advice(p)
    assert ok is False and "die" in why


def test_cannot_level():
    assert level_advice(plan(gold=5, level=None))[0] is None
    assert level_advice(plan(upgrade_cost=None))[1] == "max tier"


def test_turn_plan_buys_rolls_and_sells():
    """Plateau plein, une carte core en boutique : l'acheter, vendre le plus faible qui n'est pas protégé."""
    core, filler = _cards("Auto Reveille", "Glambot")
    p = plan(gold=4, upgrade_cost=None, level=None, team_size=7,
             per_card={core.id: sim(win=70, loss=20), filler.id: sim()},
             power_gain={core.id: 5, filler.id: 1},
             weakest=[("PROTECTED", "Pair piece", 2, False), ("BGS_004", "Wrath Weaver", 4, False)])
    out = turn_plan(p, [ShopPick(core, 3.0, ["core Mech APM"])], protect={"PROTECTED"})
    assert out["buy"][0][0] == core and "core Mech APM" in out["buy"][0][3]
    assert out["plan_buys"] == [core]
    assert out["sell"] == "Wrath Weaver (weakest)"
    assert out["roll"].startswith("Then roll 2×")          # 4 - 3 + 1 (vente) = 2 d'or qui restent


def test_turn_plan_nothing_to_buy_means_roll():
    p = plan(gold=7, upgrade_cost=None, level=None, per_card={}, power_gain={})
    out = turn_plan(p)
    assert out["buy"] == [] and "roll up to 4×" in out["roll"]


def test_danger_from_lethal():
    out = turn_plan(plan(base=sim(win=0, loss=100, taken=12, lethal=60), health=10))
    assert out["danger"][0] == "critical"


@pytest.mark.slow
def test_game1_plan_and_costs():
    """Partie du 26/10, tour 6 : prix de la montée et de la relance lus sur les boutons, plan calculé."""
    from bgtracker.planner import fight_plan
    if not GAME1.exists():
        pytest.skip("testdata/bg_game1.log absent")
    p = PowerLogParser()
    with open(GAME1, encoding="utf-8", errors="replace") as f:
        for line in f:
            p.feed_line(line)
            g = p.game
            if g and g.turn == 6 and not g.in_combat and len(g.shop) >= 3 and g.my_gold == 8:
                break
    assert g.upgrade_cost == 7 and g.reroll_cost == 1
    fp = fight_plan(g, runs=200, seconds=1)
    assert fp["gold"] == 8 and fp["opponent"] and fp["level"] is not None
    assert set(fp["per_card"]) == {get_db().base(e.card_id).id for e in g.shop}
    assert fight_plan(g) is fp                              # en cache tant que rien ne change


def test_economy_file_names_are_real_cards():
    import yaml
    from bgtracker.planner import ECONOMY_PATH
    with open(ECONOMY_PATH, encoding="utf-8") as f:
        raw = yaml.safe_load(f)
    assert len(load_economy()) == len(raw) > 40               # chaque nom est trouvé dans cards.json


def test_economy_counts_more_early():
    accord = _cards("Accord-o-Tron")[0]
    early, why = economy_value(accord.id, 3)
    assert early > economy_value(accord.id, 12)[0] > 0 and "gold" in why
    assert economy_value(_cards("Wrath Weaver")[0].id, 3) == (0.0, "")


def test_economy_card_beats_vanilla_buy():
    """Même effet en combat : la carte qui rapporte de l'or passe devant."""
    accord, vanilla = _cards("Accord-o-Tron", "Wrath Weaver")
    p = plan(turn=5, per_card={accord.id: sim(), vanilla.id: sim()}, power_gain={accord.id: 6, vanilla.id: 6})
    out = turn_plan(p)
    assert out["buy"][0][0] == accord and any("economy" in r for r in out["buy"][0][3])


def test_sell_used_battlecry_and_keep_gold_engines():
    busker, accord, weaver = _cards("Southsea Busker", "Accord-o-Tron", "Wrath Weaver")
    board = [(accord.id, "Accord-o-Tron", 4, False), (weaver.id, "Wrath Weaver", 5, False),
             (busker.id, "Southsea Busker", 6, False)]
    name, why = sell_choice(board, turn=6, tier=3)
    assert name == "Southsea Busker" and "Battlecry" in why
    assert sell_choice(board[:2], turn=6, tier=3)[0] == "Wrath Weaver"     # Accord-o-Tron rapporte encore


# ------------------------------------------------------------------ héros (data/hero_curves.yaml)
def test_hero_curves_file_names_are_real_heroes():
    import yaml
    from bgtracker.advice import load_heroes
    from bgtracker.planner import load_hero_curves
    assert set(load_hero_curves()) <= set(load_heroes())
    for name, entry in load_hero_curves().items():
        assert set(entry) <= {"curve", "power", "level_bias", "minion_cost", "buy_bonus", "pair_bonus", "note"}, name


def test_rafaam_curve_and_hero_power_gold():
    from bgtracker.planner import hero_profile
    power = [("I'll Take That!", "X", 1, False)]
    p = hero_profile("Arch-Villain Rafaam", power, turn=4)
    assert p["curve"][3] == 1 and p["curve"][4] == 2 and p["curve"][6] == 4
    assert p["reserve"] == 1 and p["power"] == "I'll Take That!"
    assert hero_profile("Arch-Villain Rafaam", [("I'll Take That!", "X", 1, True)], turn=4)["reserve"] == 0  # déjà utilisé
    std = hero_profile("Unknown Hero", [], turn=4)
    assert std["curve"][2] == 2 and std["curve"][5] == 3 and std["reserve"] == 0
    # Règle automatique : pouvoir à 1-2 d'or gardé à partir du tour 3, jamais pour "never"
    assert hero_profile("Unknown Hero", [("Paid", "Y", 2, False)], turn=3)["reserve"] == 2
    assert hero_profile("Unknown Hero", [("Paid", "Y", 2, False)], turn=2)["reserve"] == 0
    assert hero_profile("Zephrys, the Great", [("Three Wishes", "Z", 3, False)], turn=8)["reserve"] == 0


def test_hero_curve_changes_level_advice():
    from bgtracker.planner import hero_profile
    # Tour 3, Tier 1 → 2 : à l'heure sur la courbe standard, en avance sur celle de Rafaam (T2 au tour 4)
    std = plan(turn=3, tier=1, gold=5, upgrade_cost=3, budget=5, profile=hero_profile("X", [], 3))
    raf = plan(turn=3, tier=1, gold=5, upgrade_cost=3, budget=4,
               profile=hero_profile("Arch-Villain Rafaam", [("I'll Take That!", "X", 1, False)], 3))
    assert level_advice(std)[1].endswith(": on curve")
    assert "behind curve" in level_advice(plan(turn=5, tier=1, gold=5, upgrade_cost=3, budget=5))[1]
    assert "ahead of Arch-Villain Rafaam's curve" in level_advice(raf)[1]
    out = turn_plan(raf)
    assert out["power"].startswith("Use I'll Take That! (1 gold") and "Rafaam curve" in out["hero_note"]


def test_millhouse_buys_with_2_gold_minions():
    from bgtracker.planner import hero_profile
    a, b, c = _cards("Wrath Weaver", "Glambot", "Annoy-o-Module")
    p = plan(gold=6, budget=6, upgrade_cost=None, level=None, per_card={x.id: sim(win=60, loss=30) for x in (a, b, c)},
             power_gain={x.id: 4 for x in (a, b, c)}, profile=hero_profile("Millhouse Manastorm", [], 6))
    assert len(turn_plan(p)["plan_buys"]) == 3                 # 3 × 2 d'or


def test_hoggarr_likes_pirates():
    from bgtracker.planner import hero_profile
    pirate, other = _cards("Southsea Busker", "Wrath Weaver")
    p = plan(per_card={pirate.id: sim(), other.id: sim()}, power_gain={pirate.id: 4, other.id: 4},
             profile=hero_profile("Cap'n Hoggarr", [], 6))
    out = turn_plan(p)
    assert out["buy"][0][0] == pirate and any("Cap'n Hoggarr" in r for r in out["buy"][0][3])


# ------------------------------------------------------------ synergie avec le plateau actuel
def test_board_synergy_tribe_and_mechanics():
    from bgtracker.planner import board_synergy
    pup, = _cards("Wolf Pup")
    assert board_synergy(pup, _cards("Headhunter Gryphon"))[0] == 0      # Gryphon donne des Beasts, n'en profite pas
    gearfin, = _cards("Gearfin")
    pts, why = board_synergy(gearfin, _cards("Vicious Mindslasher"))
    assert pts > 0 and "Tavern spell" in why
    assert board_synergy(_cards("Wrath Weaver")[0], _cards("Annoy-o-Module"))[0] == 0


def test_synergy_counts_in_buy_value():
    gearfin, weaver = _cards("Gearfin", "Wrath Weaver")
    p = plan(per_card={gearfin.id: sim(), weaver.id: sim()}, power_gain={gearfin.id: 4, weaver.id: 4},
             team_ids=[_cards("Vicious Mindslasher")[0].id])
    out = turn_plan(p)
    assert out["buy"][0][0] == gearfin and any("synergy now" in r for r in out["buy"][0][3])


# --------------------------------------------------------- héros × comps (data/hero_comps.yaml)
def test_hero_best_comps_and_lobby_order():
    from bgtracker.comps import hero_best_comps, load_comps, load_hero_comps, lobby_comps
    comps = load_comps()
    assert set(load_hero_comps()) <= {c.id for c in comps}
    best = hero_best_comps("Vanndar Stormpike", comps)
    assert best[0][0].id == "neutral-tea-set" and best[0][1] == 2.57
    # Races connues : seulement les comps de ce lobby
    mech = hero_best_comps("Vanndar Stormpike", comps, races=["MECHANICAL", "BEAST", "DRAGON", "MURLOC", "PIRATE"])
    assert all(set(c.races) <= {"MECHANICAL", "BEAST", "DRAGON", "MURLOC", "PIRATE"} for c, _, _ in mech)
    lobby = ["MECHANICAL", "BEAST", "DRAGON", "MURLOC", "PIRATE"]
    plain = [c.id for c in lobby_comps(comps, lobby)]
    with_hero = [c.id for c in lobby_comps(comps, lobby, hero="Tras'tath, Soul Parasite")]
    assert with_hero[0] == "dragon-shield" and plain != with_hero
