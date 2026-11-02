"""Historique de mes parties (phase 6) : une ligne par partie finie, et mes propres statistiques.

Qui fait quoi :
- game_record(game, ...) résume une partie finie : date, héros, Déité, races, plateau final,
  comp reconnue sur ce plateau, place finale.
- add_records(...) ajoute des parties à history.jsonl (dossier de mes données, USER_DIR) (une partie par ligne, format JSON) sans
  doublon : une partie est reconnue par sa date, son heure de début et mon héros.
- import_logs(paths) relit de vieux Power.log pour remplir l'historique d'un coup.
- stats(records, key) : mon placement moyen par comp, par Déité, par héros...
- rule_report(records, rules) : pour chaque règle de rules.yaml, ma moyenne sur cette comp quand la
  condition était vraie, et quand elle était fausse. C'est ainsi qu'on vérifie une règle comme
  "Aberration Deathrattle si la Déité est C'Thun".
"""
import json
import re
from datetime import date
from pathlib import Path

from .cards import get_db
from .comps import match_comps
from .paths import USER_DIR
from .powerlog import PowerLogParser
from .rules import check

HISTORY_PATH = USER_DIR / "history.jsonl"
# Dossier de logs du jeu : "Hearthstone_2026_10_26_11_58_02" -> date de la session
_FOLDER_DATE = re.compile(r"Hearthstone_(\d{4})_(\d{2})_(\d{2})")


def log_date(path):
    """Date de la session de jeu d'après le dossier du log (aujourd'hui si on ne la trouve pas)."""
    m = _FOLDER_DATE.search(str(path)) if path else None
    return f"{m.group(1)}-{m.group(2)}-{m.group(3)}" if m else date.today().isoformat()


def game_record(game, comps, day):
    """Dict d'une partie Battlegrounds finie (None si la partie n'est pas finie ou pas lisible)."""
    if not (game.is_battlegrounds and game.is_over and game.my_placement and game.my_hero):
        return None
    db = get_db()
    board = [db.base(e.card_id) for e in game.my_board if e.card_id and db.base(e.card_id)]
    races = game.available_races if game.races_complete else None
    best = match_comps(comps, board, races=races, top=1)
    return {
        "date": day,
        "time": game.start_time,
        "hero": game.my_hero.name,
        "deity": game.deity,
        "races": sorted(game.available_races),
        "duos": game.is_duos,
        "placement": game.my_placement,
        "board": [c.name for c in board],
        "comp": best[0].comp.id if best else None,
        "comp_confidence": best[0].confidence if best else 0,
    }


def _key(r):
    return r["date"], r["time"], r["hero"]


def load_history(path=HISTORY_PATH):
    path = Path(path)
    if not path.exists():
        return []
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def add_records(records, path=HISTORY_PATH):
    """Ajoute les parties pas encore connues. Renvoie le nombre de parties ajoutées."""
    path = Path(path)
    known = {_key(r) for r in load_history(path)}
    new = []
    for r in records:
        if r and _key(r) not in known:
            known.add(_key(r))
            new.append(r)
    if new:
        path.parent.mkdir(parents=True, exist_ok=True)
        with open(path, "a", encoding="utf-8") as f:
            for r in new:
                f.write(json.dumps(r, ensure_ascii=False) + "\n")
    return len(new)


def import_logs(paths, comps):
    """Toutes les parties Battlegrounds finies de ces fichiers Power.log."""
    records = []
    for path in paths:
        parser = PowerLogParser()
        with open(path, encoding="utf-8", errors="replace") as f:
            parser.feed_lines(f)
        records += [game_record(g, comps, log_date(path)) for g in parser.games]
    return [r for r in records if r]


def _summary(group):
    places = [r["placement"] for r in group]
    return {"games": len(places), "avg": round(sum(places) / len(places), 2),
            "top4": round(100 * sum(p <= 4 for p in places) / len(places))}


def stats(records, key):
    """[(valeur, {"games", "avg", "top4"})] groupé par key ("comp", "deity", "hero"...), meilleur d'abord."""
    groups = {}
    for r in records:
        groups.setdefault(r.get(key) or "?", []).append(r)
    return sorted(((k, _summary(g)) for k, g in groups.items()), key=lambda kv: kv[1]["avg"])


def rule_report(records, rules):
    """Pour chaque règle : ma moyenne sur ses comps quand sa condition était vraie / fausse."""
    report = []
    for rule in rules:
        played = [r for r in records if r.get("comp") in rule.comps]
        yes, no = [], []
        for r in played:
            ok = check(rule.cond, {"races": r.get("races"), "deity": r.get("deity"), "hero": r.get("hero")})
            (yes if ok else no).append(r)
        report.append((rule, _summary(yes) if yes else None, _summary(no) if no else None))
    return report


def format_report(records, rules):
    """Texte lisible pour la console : moyennes globales, par comp, Déité, héros, puis les règles."""
    if not records:
        return "Aucune partie dans l'historique. Lancer : python -m bgtracker history --import"
    lines = ["Parties : {games} · place moyenne {avg} · top 4 : {top4} %".format(**_summary(records))]
    for key, title in (("comp", "Par comp"), ("deity", "Par Déité"), ("hero", "Par héros")):
        lines += ["", f"{title} (place moyenne, nb de parties) :"]
        lines += [f"  {k:<32} {s['avg']:.2f}  ({s['games']})" for k, s in stats(records, key)]
    if rules:
        def fmt(s):
            return f"{s['avg']:.2f} ({s['games']})" if s else "–"
        lines += ["", "Mes règles (place moyenne sur ces comps : condition vraie / fausse) :"]
        for rule, yes, no in rule_report(records, rules):
            lines.append(f"  {rule.effect} {rule.cond} → {', '.join(rule.comps)} : {fmt(yes)} / {fmt(no)}")
    return "\n".join(lines)
