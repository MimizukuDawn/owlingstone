"""Point d'entrée : `python -m bgtracker` (fenêtre), `... live` ou `... replay <fichier.log>`.

- (rien) ou ui : la fenêtre par-dessus le jeu. C'est aussi ce que lance Owlingstone.exe.

- live   : suit la partie en cours et affiche l'état à chaque changement.
- replay : relit un ancien log et affiche l'état au début de chaque phase de recrutement.
- history : mes statistiques (place moyenne par comp, Déité, héros, et pour chaque règle) ;
  `history --import [dossier ou fichier]` ajoute d'abord les parties des vieux Power.log.
"""
import argparse
import json
from pathlib import Path

from .powerlog import PowerLogParser
from .watcher import DEFAULT_LOGS_DIR, LogWatcher


def _show(summary):
    print(json.dumps(summary, ensure_ascii=False))


def replay(path):
    parser = PowerLogParser()
    last = None
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            parser.feed_line(line)
            g = parser.game
            # On affiche quand la boutique du tour vient d'apparaître (début du recrutement).
            if g and g.is_battlegrounds and g.shop and (g.turn, len(g.shop)) != last and g.turn != (last or (0,))[0]:
                last = (g.turn, len(g.shop))
                _show(g.summary())
    for i, g in enumerate(parser.games, 1):
        if g.is_battlegrounds:
            print(f"--- partie {i} finie : place {g.my_placement}, héros {g.my_hero.name if g.my_hero else '?'}, races {g.available_races}")


def live():
    last = None

    def on_update(game):
        nonlocal last
        if not game.is_battlegrounds:
            return
        s = game.summary()
        if s != last:                           # n'affiche que si l'état a changé
            last = s
            _show(s)

    print("En attente de Hearthstone... (Ctrl+C pour arrêter)")
    LogWatcher().run(on_update)


def history(import_from=None):
    from .comps import load_comps
    from .history import add_records, format_report, import_logs, load_history
    from .rules import load_rules
    comps = load_comps()
    if import_from is not None:
        src = Path(import_from)
        paths = sorted(src.glob("Hearthstone_*/Power*.log")) if src.is_dir() else [src]
        added = add_records(import_logs(paths, comps))
        print(f"{len(paths)} fichiers lus, {added} nouvelles parties dans l'historique.\n")
    print(format_report(load_history(), load_rules(comp_ids={c.id for c in comps})))


def main():
    ap = argparse.ArgumentParser(prog="bgtracker")
    sub = ap.add_subparsers(dest="cmd")       # sans commande : la fenêtre (double-clic sur l'exe)
    sub.add_parser("live")
    r = sub.add_parser("replay")
    r.add_argument("path")
    u = sub.add_parser("ui", help="fenêtre toujours au-dessus (par défaut : partie en direct)")
    u.add_argument("--replay", help="relire ce log au lieu de suivre la partie en direct")
    u.add_argument("--turn", type=int, help="avec --replay : s'arrêter à ce tour (0 = choix du héros)")
    u.add_argument("--screenshot", help="enregistrer la fenêtre en PNG puis quitter (tests)")
    h = sub.add_parser("history", help="mes statistiques de parties")
    h.add_argument("--import", dest="import_from", nargs="?", const=str(DEFAULT_LOGS_DIR),
                   help="lire d'abord les vieux logs (par défaut : le dossier Logs du jeu)")
    args = ap.parse_args()
    if args.cmd == "live":
        live()
    elif args.cmd == "history":
        history(args.import_from)
    elif args.cmd in ("ui", None):
        from .ui import run  # import tardif : PySide6 n'est chargé que pour la fenêtre
        raise SystemExit(run(getattr(args, "replay", None), getattr(args, "turn", None),
                             getattr(args, "screenshot", None)))
    else:
        replay(args.path)


if __name__ == "__main__":
    main()
