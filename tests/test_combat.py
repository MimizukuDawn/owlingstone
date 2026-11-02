"""Tests du simulateur de combat (combat.py) : règles de base, effets de cartes, et vrais combats du log."""
from bgtracker.combat import combat_info, fight_odds, simulate
from tests.test_powerlog import game1  # noqa: F401  (fixture partagée, utilisée en paramètre)


def m(card_id="", attack=1, health=1, **kw):
    return dict(card_id=card_id, attack=attack, health=health, tier=kw.pop("tier", 1), **kw)


def odds(mine, theirs, **kw):
    return simulate(mine, theirs, runs=kw.pop("runs", 400), seed=1, **kw)


def test_empty_and_bigger_board():
    r = odds([m(attack=3, health=3)], [])
    assert r["win"] == 100 and r["dealt"] == 1 + 1          # tier du héros + tier du serviteur restant
    assert odds([], [])["tie"] == 100
    assert odds([m(attack=5, health=5)], [m(attack=1, health=1)])["win"] == 100


def test_trade_is_a_tie():
    assert odds([m(attack=2, health=2)], [m(attack=2, health=2)])["tie"] == 100


def test_divine_shield_venomous_reborn():
    assert odds([m(attack=2, health=2, ds=True)], [m(attack=2, health=2)])["win"] == 100
    assert odds([m(attack=1, health=1, venom=True)], [m(attack=1, health=50)])["tie"] == 100
    # Réincarnation : revient avec 1 PV et gagne le combat
    theirs = [m(attack=1, health=3, taunt=True), m(attack=0, health=3)]
    assert odds([m(attack=3, health=1)], theirs)["loss"] == 100
    assert odds([m(attack=3, health=1, reborn=True)], theirs)["win"] == 100


def test_taunt_is_attacked_first():
    # Mon 4/4 frappe toujours la Provocation 3/10 (et meurt au 2e coup), jamais le 0/1
    r = odds([m(attack=4, health=4)], [m(attack=3, health=10, taunt=True), m(attack=0, health=1)])
    assert r["loss"] == 100


def test_deathrattle_summons():
    vermin = m("BG31_803", attack=1, health=1, taunt=True)   # Râle : invoque un Beetle 2/2
    r = odds([vermin], [m(attack=2, health=2)])
    assert r["tie"] == 100                                    # le Beetle et le 2/2 s'entretuent
    r = odds([vermin], [m(attack=2, health=2)], counters=({"beetle": (2, 1)}, {}))
    assert r["win"] == 100                                    # Beetle 4/3 grâce au compteur de la partie


def test_damage_cap_and_lethal():
    big = [m(attack=10, health=10, tier=6) for _ in range(7)]
    r = simulate(big, [], my_tier=6, their_health=20, runs=50, seed=1)
    assert r["dealt"] == 48 and r["lethal"] == 100
    r = simulate(big, [], my_tier=6, their_health=20, cap=15, runs=50, seed=1)
    assert r["dealt"] == 15 and r["lethal"] == 0


def test_game1_fights_are_predicted(game1):  # noqa: F811
    # Les 10 combats de la partie : le résultat le plus probable selon la simulation est le bon.
    for f in game1.fights:
        r = fight_odds(game1, f, runs=500, seconds=2)
        best = max(("W", r["win"]), ("T", r["tie"]), ("L", r["loss"]), key=lambda x: x[1])[0]
        assert best == f["result"], (f["turn"], r)


def test_game1_combat_info(game1):  # noqa: F811
    info = combat_info(game1)
    assert len(info["games_fights"]) == 10
    assert info["actual_wins"] == 2 and 0 <= info["avg_win"] <= 100
    assert "now" not in info or not info["now"]["live"]
