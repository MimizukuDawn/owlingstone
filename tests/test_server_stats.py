"""Raisons probables des abandons précoces (server/build_stats.py), publiées seulement en totaux."""
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location("build_stats", Path(__file__).parent.parent / "server" / "build_stats.py")
build_stats = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build_stats)


def fight(t, r, d, tier=3, otier=3, me=10, opp=10):
    return {"t": t, "tier": tier, "otier": otier, "r": r, "d": d, "me": [{"a": me, "h": 0}], "opp": [{"a": opp, "h": 0}]}


def test_tilt_after_two_big_losses():
    g = {"hero": "X", "comp": "undead-apm", "end_hp": 20, "actions": [2, 3, 4, 5, 6],
         "fights": [fight(4, "W", 3), fight(5, "L", 12), fight(6, "L", 9)]}
    assert build_stats.concede_reasons(g) == ["lost_streak", "big_hit", "left_after_a_loss"]


def test_behind_on_tier_with_weak_board_and_slow_leveling():
    g = {"hero": "X", "comp": None, "end_hp": 8, "actions": [2, 3, 4, 5, 6],
         "fights": [fight(5, "W", 2, tier=2, otier=4, me=10, opp=30)]}
    assert build_stats.concede_reasons(g) == ["behind_on_tier", "weak_board", "slow_leveling", "no_comp"]


def test_report_only_publishes_groups_with_enough_games():
    g = {"hero": "Rare hero", "comp": None, "end_hp": 20, "actions": [1], "fights": []}
    report = build_stats.concede_report([{"summary": json.dumps(g)}] * 3)
    assert report["games"] == 3 and report["reasons"] == {"no_comp": 3} and report["by_hero"] == {}
