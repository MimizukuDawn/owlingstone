"""Lanceur de la fenêtre sans console : double-clic (pythonw) et point d'entrée de Owlingstone.exe."""
import sys
import traceback

from bgtracker.paths import USER_DIR


def log_crash(kind, value, tb):
    """Sans console, une erreur serait invisible : on l'écrit dans error.log (dossier de mes données)."""
    try:
        USER_DIR.mkdir(parents=True, exist_ok=True)
        with open(USER_DIR / "error.log", "a", encoding="utf-8") as f:
            f.write("".join(traceback.format_exception(kind, value, tb)) + "\n")
    except OSError:
        pass
    sys.__excepthook__(kind, value, tb)


if __name__ == "__main__":
    sys.excepthook = log_crash
    from bgtracker.__main__ import main
    main()
