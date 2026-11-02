"""Tests des conseils de la phase 5 : choix du héros et achats dans la boutique."""
import pytest

from bgtracker.advice import load_heroes, non_golden, rank_heroes, shop_advice, target_comps
from bgtracker.cards import get_db
from bgtracker.comps import load_comps
from bgtracker.powerlog import PowerLogParser
from tests.test_comps import _cards
from tests.test_powerlog import GAME1


def test_heroes_file_matches_card_names():
    """Chaque héros de heroes.yaml porte un nom exact de cards.json (sinon il ne serait jamais trouvé)."""
    names = {c.name for c in get_db().by_id.values() if c.is_hero}
    heroes = load_heroes()
    assert len(heroes) > 100
    assert set(heroes) <= names


def test_rank_heroes_best_first_unknown_last():
    heroes = {"A": {"avg": 4.1}, "B": {"avg": 3.6}}
    assert [n for n, _ in rank_heroes(["A", "X", "B"], heroes)] == ["B", "A", "X"]


def test_game1_hero_options_follow_reroll_and_pick():
    """Partie du 26/10 : 4 héros proposés, Dancin' Deryl relancé en Thorim, puis Thorim choisi."""
    if not GAME1.exists():
        pytest.skip("testdata/bg_game1.log absent")
    p = PowerLogParser()
    seen = []
    with open(GAME1, encoding="utf-8", errors="replace") as f:
        for i, line in enumerate(f):
            p.feed_line(line)
            if i % 50 == 0 and p.game:
                opts = sorted(e.name for e in p.game.hero_options)
                if opts and opts not in seen:
                    seen.append(opts)
            if i > 3000:
                break
    assert seen[-2:] == [["Dancin' Deryl", "Genn, Worgen King", "George the Fallen", "Rock Master Voone"],
                         ["Genn, Worgen King", "George the Fallen", "Rock Master Voone", "Thorim, Stormlord"]]
    assert p.game.hero_options == []          # héros choisi
    assert p.game.my_hero.name == "Thorim, Stormlord"


def test_shop_advice_core_addon_triple_pair():
    comps = load_comps()
    apm = next(c for c in comps if c.id == "mech-apm")
    shop = _cards("Auto Reveille", "Drone Duplicator", "Annoy-o-Module", "Wrath Weaver", "Glambot")
    owned = _cards("Annoy-o-Module", "Annoy-o-Module", "Glambot")
    picks = shop_advice(shop, owned, [(apm, 1.0)])
    assert [p.card.name for p in picks] == ["Auto Reveille", "Annoy-o-Module", "Drone Duplicator", "Glambot"]
    assert picks[0].reasons == ["core Mech APM"]
    assert picks[1].reasons == ["triple"]
    assert picks[3].reasons == ["pair"]       # Glambot : pas dans Mech APM, mais j'en ai déjà 1


def test_golden_copies_do_not_make_triples():
    two = _cards("Annoy-o-Module", "Annoy-o-Module")
    owned = non_golden(two, [two[0].id])      # un des deux est doré
    assert len(owned) == 1
    assert shop_advice(_cards("Annoy-o-Module"), owned, [])[0].reasons == ["pair"]


def test_target_comps_lobby_then_playing():
    comps = load_comps()
    lobby = ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"]
    early = target_comps(comps, [], races=lobby)
    assert [w for _, w in early[:4]] == [0.5, 0.5, 0.5, 0.25]
    later = target_comps(comps, _cards("Auto Reveille", "Spark Snapper"), races=lobby)
    assert later[0] == (next(c for c in comps if c.id == "mech-apm"), 1.0)
