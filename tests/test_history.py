"""Tests de l'historique des parties (phase 6)."""
import pytest

from bgtracker.comps import load_comps
from bgtracker.history import (add_records, format_report, import_logs, load_history, log_date,
                               rule_report, stats)
from bgtracker.rules import Rule
from tests.test_powerlog import GAME1


def _rec(placement, comp, deity="C'Thun", hero="Thorim, Stormlord", time="12:00:00"):
    return {"date": "2026-10-26", "time": time, "hero": hero, "deity": deity, "placement": placement,
            "races": ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"], "comp": comp}


def test_log_date_from_folder():
    assert log_date(r"C:\Logs\Hearthstone_2026_10_26_11_58_02\Power.log") == "2026-10-26"


def test_game1_record():
    if not GAME1.exists():
        pytest.skip("testdata/bg_game1.log absent")
    [r] = import_logs([GAME1], load_comps())
    assert (r["hero"], r["placement"], r["deity"], r["time"]) == ("Thorim, Stormlord", 6, "C'Thun", "12:00:41")
    assert r["comp"] == "mech-apm"
    assert r["races"] == ["ABERRATION", "BEAST", "DRAGON", "MECHANICAL", "MURLOC"]


def test_add_records_skips_duplicates(tmp_path):
    path = tmp_path / "history.jsonl"
    assert add_records([_rec(1, "a"), _rec(2, "b", time="13:00:00")], path) == 2
    assert add_records([_rec(1, "a"), None, _rec(5, "c", time="14:00:00")], path) == 1
    assert [r["placement"] for r in load_history(path)] == [1, 2, 5]


def test_stats_and_rule_report():
    records = [_rec(1, "aberration-deathrattle"), _rec(3, "aberration-deathrattle"),
               _rec(7, "aberration-deathrattle", deity="Y'Shaarj"), _rec(4, "mech-apm")]
    by_comp = dict(stats(records, "comp"))
    assert by_comp["aberration-deathrattle"] == {"games": 3, "avg": 3.67, "top4": 67}
    assert stats(records, "deity")[0][0] == "C'Thun"          # meilleur d'abord
    rule = Rule(["aberration-deathrattle"], "prefer_if", {"deity": "C'Thun"})
    [(_, yes, no)] = rule_report(records, [rule])
    assert (yes["avg"], yes["games"], no["avg"], no["games"]) == (2.0, 2, 7.0, 1)
    text = format_report(records, [rule])
    assert "2.00 (2) / 7.00 (1)" in text


def test_empty_history_explains_import():
    assert "--import" in format_report([], [])
