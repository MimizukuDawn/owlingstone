"""build_stats.py queries on an in-memory SQLite copy of the D1 schema: one install that plays a lot
weighs PER_INSTALL games at most.    python -m unittest discover -s server/test"""
import json
import sqlite3
import sys
import unittest
from pathlib import Path

SERVER = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SERVER))
import build_stats  # noqa: E402


def fight(t):
    return {"t": t, "r": "W", "d": 3, "tier": 2, "otier": 3, "me": [{"a": 2, "h": 3}],
            "opp": [{"a": 1, "h": 1}, {"a": 2, "h": 2}]}


class Queries(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:")
        self.db.row_factory = sqlite3.Row
        self.db.executescript((SERVER / "schema.sql").read_text(encoding="utf-8"))
        games = [("busy", 1)] * 100 + [(f"p{i}", 8) for i in range(30)]
        for install, place in games:
            self.db.execute("INSERT INTO games (install, day, build, mode, hero, place, comp, quality, summary) "
                            "VALUES (?, date('now'), '2.0.0', 'solo', 'Hero', ?, 'comp_x', 'ok', ?)",
                            (install, place, json.dumps({"fights": [fight(1), fight(2)]})))

    def run_query(self, name):
        sql = build_stats.QUERIES[name].format(w=build_stats.WINDOW_DAYS)
        return [dict(r) for r in self.db.execute(sql)]

    def test_one_install_is_capped(self):
        (row,) = self.run_query("comps")
        cap = build_stats.PER_INSTALL
        self.assertEqual(row["raw_games"], 130)
        self.assertEqual(row["installs"], 31)
        self.assertAlmostEqual(row["games"], 30 + cap)
        self.assertAlmostEqual(row["avg_place"], (cap * 1 + 30 * 8) / (30 + cap))

    def test_every_query_runs(self):
        for name in build_stats.QUERIES:
            rows = self.run_query(name)
            self.assertTrue(rows, name)
        turns = {r["key"]: r for r in self.run_query("board_by_turn")}
        self.assertEqual(sorted(turns), [1, 2])
        self.assertAlmostEqual(turns[1]["avg_stats"], 6)
        self.assertAlmostEqual(turns[1]["avg_tier"], 3)


if __name__ == "__main__":
    unittest.main()
