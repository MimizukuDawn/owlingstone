"""Refait data/trinkets.yaml : stats des Trinkets (bgbuddy.gg) + infos des cartes (HearthstoneJSON).

Usage :  python tools/make_trinkets.py [tools/bgbuddy_trinkets.txt] [chemin de cards.json]

tools/bgbuddy_trinkets.txt = une ligne par Trinket de https://bgbuddy.gg/trinkets (Solo, tous rangs, patch actuel),
au format  nom|L ou G|placement moyen|% de choix|parties  (L = Lesser, G = Greater). À recopier à chaque patch.
Les Trinkets absents de la page ne sont pas dans le pool actuel : ils ne sont pas écrits.
"""
import json
import sys
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
SRC = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "tools" / "bgbuddy_trinkets.txt"
CARDS = Path(sys.argv[2]) if len(sys.argv) > 2 else ROOT / "data" / "cards.json"
OUT = ROOT / "data" / "trinkets.yaml"

HEADER = """\
# Trinkets du pool actuel : stats + infos des cartes. Fichier refait par tools/make_trinkets.py (ne pas modifier
# à la main : modifier tools/bgbuddy_trinkets.txt puis relancer l'outil).
#
# Source des stats : https://bgbuddy.gg/trinkets (données communautaires Firestone, Solo, tous rangs, patch actuel,
# relevé le 02/11/2026, saison 14, patch 36.6.x).
#   avg   = placement moyen des joueurs qui l'ont pris (plus bas = mieux)
#   pick  = % de fois où il est choisi quand il est proposé
#   games = nombre de parties (moins de 500 = chiffre peu fiable)
# Champs des cartes : kind (lesser / greater), cost (prix de base en or ; -2 si c'est un Trinket "pivot"),
# races (race liée : proposé surtout si on en a sur le plateau), gives (serviteur donné par un Portrait / Sticker).
"""


def main():
    cards = json.loads(CARDS.read_text(encoding="utf-8"))
    by_dbf = {c.get("dbfId"): c for c in cards}
    trinkets = {}
    for c in cards:
        if c.get("type") == "BATTLEGROUND_TRINKET" and not c.get("isBattlegroundsDuosExclusive"):
            trinkets[(c["name"], c["spellSchool"][0])] = c
    out = []
    for line in SRC.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        name, kind, avg, pick, games = line.split("|")
        c = trinkets.get((name, kind))
        if c is None:
            print("not found:", name, kind)
            continue
        entry = {"id": c["id"], "name": name, "kind": "lesser" if kind == "L" else "greater", "cost": c.get("cost", 0),
                 "avg": float(avg), "pick": float(pick), "games": int(games)}
        if c.get("battlegroundsAssociatedRaces"):
            entry["races"] = c["battlegroundsAssociatedRaces"]
        rel = by_dbf.get(c.get("battlegroundsRelatedCard"))
        if rel and rel.get("type") == "MINION":
            entry["gives"] = rel["name"]
        out.append(entry)
    with open(OUT, "w", encoding="utf-8") as f:
        f.write(HEADER)
        yaml.safe_dump(out, f, allow_unicode=True, sort_keys=False, width=200)
    print(len(out), "trinkets ->", OUT)


if __name__ == "__main__":
    main()
