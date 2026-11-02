"""Builds stats.json from the shared games in D1 (run once a day by .github/workflows/stats.yml).

    python server/build_stats.py --remote out/stats.json      (or --local for `wrangler dev` tests)

Each query aggregates in the database, so the job reads each game once per query and downloads only
a few hundred numbers. Each install weighs PER_INSTALL games at most in a group, and only groups with
at least MIN_GAMES (weighted) games are published.
It also deletes games older than KEEP_DAYS days so the database stays far below the 5 GB free limit.
"""
import json
import subprocess  # nosec B404 : runs the wrangler CLI with fixed arguments
import sys
from datetime import datetime, timezone

WINDOW_DAYS = 30      # stats use the last 30 days (about one patch)
KEEP_DAYS = 60
FEEDBACK_DAYS = 180     # feedback the maintainer has handled is deleted after this many days
MIN_GAMES = int(__import__("os").environ.get("MIN_GAMES", 30))

# One player who plays a lot must not make the stats alone: in each group (a comp, a hero...), each
# install counts for PER_INSTALL games at most. Its games are weighted down to that (weight w), "games" is
# the weighted count (used for MIN_GAMES), "raw_games" the real one, "installs" how many players.
PER_INSTALL = int(__import__("os").environ.get("PER_INSTALL", 10))
SOLO_OK = "mode = 'solo' AND quality = 'ok' AND day >= date('now', '-{w} day')"


def grouped(key, group, where=SOLO_OK):
    """Average place per group, each install's weight capped at PER_INSTALL games in the group."""
    return f"""SELECT {key} AS key, SUM(w) AS games, COUNT(*) AS raw_games, COUNT(DISTINCT install) AS installs,
                      SUM(place * w) / SUM(w) AS avg_place
               FROM (SELECT *, MIN(1.0, {PER_INSTALL}.0 / COUNT(*) OVER (PARTITION BY {group}, install)) AS w
                     FROM games WHERE {where})
               GROUP BY {group}"""


def by_turn(columns, where):
    """Per-turn averages over the fights; each install's weight capped at PER_INSTALL games per turn."""
    return f"""SELECT t AS key, SUM(w) AS games, COUNT(*) AS raw_games, COUNT(DISTINCT install) AS installs, {columns}
               FROM (SELECT *, MIN(1.0, {PER_INSTALL}.0 / COUNT(*) OVER (PARTITION BY t, install)) AS w
                     FROM (SELECT g.install, CAST(json_extract(f.value, '$.t') AS INTEGER) AS t, f.value AS fight
                           FROM games g, json_each(g.summary, '$.fights') f WHERE {where}))
               GROUP BY t"""


QUERIES = {
    "comps": grouped("comp", "comp", SOLO_OK + " AND comp IS NOT NULL"),
    "heroes": grouped("hero", "hero"),
    "hero_comps": grouped("hero || ' / ' || comp", "hero, comp", SOLO_OK + " AND comp IS NOT NULL"),
    "board_by_turn": by_turn(
        """SUM(w * (SELECT TOTAL(json_extract(m.value, '$.a') + json_extract(m.value, '$.h'))
                    FROM json_each(fight, '$.opp') m)) / SUM(w) AS avg_stats,
           SUM(w * json_extract(fight, '$.otier')) / SUM(w) AS avg_tier,
           SUM(w * json_array_length(fight, '$.opp')) / SUM(w) AS avg_minions""",
        "g.mode = 'solo' AND g.quality = 'ok' AND g.day >= date('now', '-{w} day') "
        "AND (json_extract(f.value, '$.t') <= 3 OR json_array_length(f.value, '$.opp') > 0)"),
    "tier_by_turn_top4": by_turn(
        "SUM(w * json_extract(fight, '$.tier')) / SUM(w) AS avg_tier",
        "g.mode = 'solo' AND g.quality = 'ok' AND g.place <= 4 AND g.day >= date('now', '-{w} day')"),
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
             "min_games": MIN_GAMES, "per_install": PER_INSTALL}
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
    d1(f"DELETE FROM feedback WHERE done = 1 AND day < date('now', '-{FEEDBACK_DAYS} day')", where)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(stats, f, indent=1, sort_keys=True)
    print(f"{stats['total_games']} games -> {path}")


if __name__ == "__main__":
    main(sys.argv[1:])
