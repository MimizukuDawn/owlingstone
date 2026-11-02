"""Tests de la liste par tier (pool.py) contre une vraie partie."""
from collections import defaultdict

import pytest

from bgtracker.pool import card_groups, is_available, learn_subsets, tier_list
from tests.test_powerlog import GAME1, _parse


@pytest.fixture(scope="module")
def parsed():
    if not GAME1.exists():
        pytest.skip("testdata/bg_game1.log absent")
    return _parse(GAME1)


def test_every_shop_minion_is_listed(parsed):
    """Tout serviteur que Bob a proposé doit figurer dans la liste filtrée par les races de la partie."""
    g = parsed.game
    subsets = learn_subsets(parsed.games)
    listed = defaultdict(set)
    for tier in range(1, 8):
        for _, cards in tier_list(tier, g.available_races, subsets):
            listed[tier].update(c.id for c in cards)
    for eid in g._seen_in_shop:
        card = g.entities[eid].card
        assert card.id in listed[card.tier], f"{card.name} (tier {card.tier}) absent de la liste"


def test_off_lobby_race_hidden(parsed):
    """Un serviteur à race simple hors partie (ex. Pirate) ne doit pas apparaître."""
    g = parsed.game
    names = {c.name for _, cards in tier_list(1, g.available_races, {}) for c in cards}
    # Aucun Pirate/Démon/Elementaire/Naga/Quilboar/Mort-vivant à race simple au tier 1
    from bgtracker.cards import get_db
    for c in get_db().pool_minions():
        if c.tier == 1 and len(c.races) == 1 and c.races[0] not in g.available_races + ["ALL"]:
            assert c.name not in names


def test_dual_race_rules():
    assert is_available(["DEMON", "DRAGON"], ["DRAGON", "BEAST"])
    assert not is_available(["DEMON", "PIRATE"], ["DRAGON", "BEAST"])
    assert is_available(["NEUTRAL"], ["DRAGON"])
    assert is_available(["UNDEAD"], None)  # races inconnues : tout est montré


def test_subset_neutral_grouped_by_race():
    from bgtracker.cards import get_db
    graverobber = next(c for c in get_db().pool_minions() if c.name == "Disguised Graverobber")
    assert card_groups(graverobber, {graverobber.id: ["UNDEAD"]}) == ["UNDEAD"]
