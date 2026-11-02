"""Tests des règles de préférence (rules.py et data/rules.yaml)."""
import pytest

from bgtracker.comps import load_comps
from bgtracker.rules import Rule, check, judge, load_rules


def test_rules_file_valid():
    """data/rules.yaml se lit et ne vise que des comps qui existent."""
    load_rules(comp_ids={c.id for c in load_comps()})


def test_bad_rules_are_reported(tmp_path):
    f = tmp_path / "rules.yaml"
    f.write_text("- comp: nope\n  only_if: {lobby_has: [PIRATE]}\n"
                 "- comp: aberration-apm\n  only_if: {}\n  never_if: {}\n"
                 "- comp: aberration-apm\n  prefer_if: {lobby: [PIRATE]}\n", encoding="utf-8")
    with pytest.raises(ValueError) as e:
        load_rules(f, comp_ids={"aberration-apm"})
    msg = str(e.value)
    assert "comp inconnue 'nope'" in msg and "un seul effet" in msg and "condition inconnue 'lobby'" in msg


def test_check_conditions():
    ctx = {"races": ["ABERRATION", "PIRATE", "BEAST"], "deity": "C'Thun", "hero": "Sir Finley"}
    assert check({"lobby_has": ["PIRATE", "BEAST"]}, ctx) is True
    assert check({"lobby_has": "DRAGON"}, ctx) is False
    assert check({"lobby_has_any": ["DEMON", "BEAST"]}, ctx) is True
    assert check({"lobby_lacks": ["BEAST"]}, ctx) is False
    assert check({"deity": "c'thun", "hero": "Sir Finley"}, ctx) is True
    assert check({}, ctx) is True
    assert check({"lobby_has": ["PIRATE"]}, {"races": None}) is None   # races pas encore sûres
    assert check({"lobby_has": ["DRAGON"], "deity": "C'Thun"}, {"races": None, "deity": "Y'Shaarj"}) is False


def test_judge_combines_rules():
    rules = [Rule(["a"], "only_if", {"lobby_has": ["PIRATE"]}),
             Rule(["a", "b"], "prefer_if", {"deity": "C'Thun"}, bonus=2, note="C'Thun!"),
             Rule(["a"], "avoid_if", {"deity": "C'Thun"}, bonus=5)]
    v = judge("a", rules, {"races": ["PIRATE"], "deity": "C'Thun"})
    assert not v.hidden and v.bonus == -3 and v.notes == ["C'Thun!"]
    assert judge("a", rules, {"races": ["BEAST"], "deity": None}).hidden
    assert not judge("a", rules, {"races": None, "deity": None}).hidden
    assert judge("c", rules, {"races": [], "deity": "C'Thun"}).bonus == 0


def test_max_example_rules():
    """Les 2 exemples de data/rules.yaml."""
    rules = load_rules()
    ctx = {"races": ["ABERRATION", "BEAST", "DRAGON", "MURLOC", "MECHANICAL"]}
    assert judge("aberration-apm", rules, ctx).hidden
    assert judge("aberration-deathrattle", rules, {**ctx, "deity": "C'Thun"}).bonus > 0
    assert judge("aberration-deathrattle", rules, {**ctx, "deity": "Y'Shaarj"}).bonus < 0


def test_hero_power_condition():
    from bgtracker.rules import check
    assert check({"hero_power": "Choose Your Champion"}, {"hero_power": ["Choose Your Champion"]}) is True
    assert check({"hero_power": ["X", "choose your champion"]}, {"hero_power": ["Choose Your Champion"]}) is True
    assert check({"hero_power": "X"}, {"hero_power": ["Choose Your Champion"]}) is False
    assert check({"hero_power": "X"}, {"hero_power": None}) is None
