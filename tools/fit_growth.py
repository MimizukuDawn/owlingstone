"""Réajuste la courbe de croissance des plateaux (planner.GROWTH) sur des Power.log : à relancer à chaque patch.

Usage : .venv\\Scripts\\python tools\\fit_growth.py "C:\\Program Files (x86)\\Hearthstone\\Logs\\Hearthstone_*\\Power*.log"
Affiche a, b, c de log(attaque + vie du plateau adverse) = a + b × tour + c × tier, l'écart type, et le nombre
moyen de serviteurs par tour (BOARD_COUNT)."""
import glob
import math
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from bgtracker.planner import board_power  # noqa: E402
from bgtracker.powerlog import PowerLogParser  # noqa: E402


def fights(paths):
    for path in paths:
        p = PowerLogParser()
        with open(path, encoding="utf-8", errors="replace") as f:
            p.feed_lines(f)
        for g in p.games:
            if g.is_battlegrounds and not g.is_duos:
                yield from (f for f in g.fights if "my_board" in f and f["board"])


def least_squares(rows):
    """Moindres carrés à 3 inconnues (équations normales), sans numpy."""
    xs = [(1.0, t, tier) for t, tier, _ in rows]
    a = [[sum(x[i] * x[j] for x in xs) for j in range(3)] for i in range(3)]
    b = [sum(x[i] * y for x, (_, _, y) in zip(xs, rows)) for i in range(3)]
    for i in range(3):                                   # Gauss
        for k in range(i + 1, 3):
            r = a[k][i] / a[i][i]
            a[k] = [u - r * v for u, v in zip(a[k], a[i])]
            b[k] -= r * b[i]
    coef = [0.0] * 3
    for i in reversed(range(3)):
        coef[i] = (b[i] - sum(a[i][j] * coef[j] for j in range(i + 1, 3))) / a[i][i]
    return coef


if __name__ == "__main__":
    paths = [p for arg in sys.argv[1:] for p in glob.glob(arg)]
    rows, counts = [], defaultdict(list)
    for f in fights(paths):
        rows.append((f["turn"], f["opp_tier"], math.log(max(board_power(f["board"]), 1))))
        counts[f["turn"]].append(len(f["board"]))
    coef = least_squares(rows)
    sd = math.sqrt(sum((y - coef[0] - coef[1] * t - coef[2] * tier) ** 2 for t, tier, y in rows) / len(rows))
    print(f"{len(rows)} fights · GROWTH = ({coef[0]:.2f}, {coef[1]:.3f}, {coef[2]:.3f}) · GROWTH_SD = {sd:.2f}")
    print("BOARD_COUNT =", {t: round(sum(v) / len(v)) for t, v in sorted(counts.items()) if t <= 8})
