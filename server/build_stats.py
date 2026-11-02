"""Builds stats.json from the shared games in D1 (run once a day by .github/workflows/stats.yml).

    python server/build_stats.py --remote out/stats.json      (or --local for `wrangler dev` tests)

Each query aggregates in the database, so the job reads each game once per query and downloads only
a few hundred numbers. Only groups with at least MIN_GAMES games are published.
It also deletes games older than KEEP_DAYS days so the database stays far below the 5 GB free limit.
"""
import json
import subprocess  # nosec B404 : runs the wrangler CLI with fixed arguments
import sys
from datetime import datetime, timezone

WINDOW_DAYS = 30      # stats use the last 30 days (about one patch)
KEEP_DAYS = 60
MIN_GAMES = int(__import__("os").environ.get("MIN_GAMES", 30))

QUERIES = {
    "comps": """SELECT comp AS key, COUNT(*) AS games, AVG(place) AS avg_place FROM games
                WHERE mode = 'solo' AND quality = 'ok' AND comp IS NOT NULL AND day >= date('now', '-{w} day') GROUP BY comp""",
    "heroes": """SELECT hero AS key, COUNT(*) AS games, AVG(place) AS avg_place FROM games
                 WHERE mode = 'solo' AND quality = 'ok' AND day >= date('now', '-{w} day') GROUP BY hero""",
    "hero_comps": """SELECT hero || ' / ' || comp AS key, COUNT(*) AS games, AVG(place) AS avg_place FROM games
                     WHERE mode = 'solo' AND quality = 'ok' AND comp IS NOT NULL AND day >= date('now', '-{w} day')
                     GROUP BY hero, comp""",
    "board_by_turn": """SELECT CAST(json_extract(f.value, '$.t') AS INTEGER) AS key, COUNT(*) AS games,
                          AVG((SELECT TOTAL(json_extract(m.value, '$.a') + json_extract(m.value, '$.h'))
                               FROM json_each(f.value, '$.opp') m)) AS avg_stats,
                          AVG(json_extract(f.value, '$.otier')) AS avg_tier,
                          AVG(json_array_length(f.value, '$.opp')) AS avg_minions
                        FROM games g, json_each(g.summary, '$.fights') f
                        WHERE g.mode = 'solo' AND g.quality = 'ok' AND g.day >= date('now', '-{w} day')
                          AND (json_extract(f.value, '$.t') <= 3 OR json_array_length(f.value, '$.opp') > 0)
                        GROUP BY key""",
    "tier_by_turn_top4": """SELECT CAST(json_extract(f.value, '$.t') AS INTEGER) AS key, COUNT(*) AS games,
                              AVG(json_extract(f.value, '$.tier')) AS avg_tier
                            FROM games g, json_each(g.summary, '$.fights') f
                            WHERE g.mode = 'solo' AND g.quality = 'ok' AND g.place <= 4 AND g.day >= date('now', '-{w} day')
                            GROUP BY key""",
}


def _stats(board):
    return sum(m["a"] + m["h"] for m in board)


def concede_reasons(g):
    """Likely reasons behind an early concede, read from the summary (several can apply).
    Only totals are published, never a single game."""
    fights = sorted(g.get("fights") or [], key=lambda f: f["t"])
    last = fights[-3:]
    reasons = []
    if len(last) >= 2 and all(f["r"] == "L" for f in last[-2:]):
        reasons.append("lost_streak")                        # lost the last 2+ fights
    if any(f["r"] == "L" and f["d"] >= 10 for f in last):
        reasons.append("big_hit")                            # took 10+ damage in one of the last fights
    if fights and fights[-1]["tier"] < fights[-1]["otier"]:
        reasons.append("behind_on_tier")                     # lower tavern tier than the opponent
    if fights and _stats(fights[-1]["me"]) < 0.5 * max(1, _stats(fights[-1]["opp"])):
        reasons.append("weak_board")                         # half the opponent's stats or less
    if any(f["t"] >= 5 and f["tier"] <= 2 for f in fights):
        reasons.append("slow_leveling")                      # still tier 2 or less on turn 5+
    if not g.get("comp"):
        reasons.append("no_comp")                            # no comp recognised on the final board
    acts = g.get("actions") or []
    if len(acts) >= 4 and sum(acts[1:4]) <= 3:
        reasons.append("passive_start")                      # almost nothing done on turns 2-4
    if fights and fights[-1]["r"] == "L" and g.get("end_hp", 0) >= 15:
        reasons.append("left_after_a_loss")                  # quit right after losing, still healthy
    return reasons or ["unclear"]


def concede_report(rows):
    """Totals for early concedes: how often each reason, and the top reasons per hero and per comp."""
    total, by_hero, by_comp = {}, {}, {}
    for r in rows:
        g = json.loads(r["summary"])
        for reason in concede_reasons(g):
            total[reason] = total.get(reason, 0) + 1
            by_hero.setdefault(g["hero"], {}).setdefault(reason, 0)
            by_hero[g["hero"]][reason] += 1
            if g.get("comp"):
                by_comp.setdefault(g["comp"], {}).setdefault(reason, 0)
                by_comp[g["comp"]][reason] += 1
    top = lambda d: {k: dict(sorted(v.items(), key=lambda kv: -kv[1])[:3]) for k, v in d.items()
                     if sum(v.values()) >= MIN_GAMES}
    return {"games": len(rows), "reasons": dict(sorted(total.items(), key=lambda kv: -kv[1])),
            "by_hero": top(by_hero), "by_comp": top(by_comp)}


def d1(sql, where):
    out = subprocess.run(["npx", "--yes", "wrangler@4.156.2", "d1", "execute", "owlingstone", where, "--json",
                          "--command", sql], check=True, capture_output=True, text=True).stdout  # nosec B603 B607
    return json.loads(out)[0]["results"]


def build(where):
    stats = {"generated": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%MZ"), "window_days": WINDOW_DAYS,
             "min_games": MIN_GAMES}
    for name, sql in QUERIES.items():
        rows = d1(sql.format(w=WINDOW_DAYS), where)
        stats[name] = {str(r.pop("key")): {k: round(v, 3) if isinstance(v, float) else v for k, v in r.items()}
                       for r in rows if r["games"] >= MIN_GAMES and r["key"] is not None}
    quality = d1(f"SELECT quality AS key, COUNT(*) AS games FROM games WHERE day >= date('now', '-{WINDOW_DAYS} day') "
                 "GROUP BY quality", where)
    stats["games_by_quality"] = {r["key"]: r["games"] for r in quality}
    early = d1(f"SELECT summary FROM games WHERE quality = 'early_concede' AND mode = 'solo' "
               f"AND day >= date('now', '-{WINDOW_DAYS} day') ORDER BY id DESC LIMIT 5000", where)
    stats["early_concedes"] = concede_report(early)
    stats["total_games"] = d1(f"SELECT COUNT(*) AS n FROM games WHERE day >= date('now', '-{WINDOW_DAYS} day')",
                              where)[0]["n"]
    return stats


def main(argv):
    where = "--local" if "--local" in argv else "--remote"
    path = [a for a in argv if not a.startswith("--")][0]
    stats = build(where)
    d1(f"DELETE FROM games WHERE day < date('now', '-{KEEP_DAYS} day')", where)
    d1(f"DELETE FROM day_count WHERE day < date('now', '-{KEEP_DAYS} day')", where)
    d1("DELETE FROM install_count WHERE day < date('now', '-2 day')", where)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(stats, f, indent=1, sort_keys=True)
    print(f"{stats['total_games']} games -> {path}")


if __name__ == "__main__":
    main(sys.argv[1:])
