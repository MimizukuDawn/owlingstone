"""Partage anonyme des parties et avis des joueurs (opt-in), vers l'API Cloudflare du projet (server/).

Qui fait quoi :
- state() / set_enabled(on) : le choix du joueur ("Share my games"), demandé une fois, NON par défaut.
  Un identifiant d'installation au hasard (uuid4) est créé à ce moment ; il n'est lié à aucun compte.
- game_summary(game, record) : le résumé d'une partie finie, réduit aux champs listés dans PRIVACY.md
  (héros, Déité, races, place, comp, plateau final et combats : cartes et stats). Jamais de BattleTag,
  de nom d'adversaire, de chemin de fichier ni de log.
- enqueue(...) met un résumé ou un avis dans une file locale (share_queue.jsonl) ; flush() envoie la file
  en UNE requête (jusqu'à 20 parties + 5 avis) à la fin d'une partie et au démarrage : peu d'envois, pour
  rester dans l'offre gratuite de Cloudflare. Hors ligne, la file attend (30 jours au plus).
- delete_remote() efface sur le serveur tout ce que cette installation a envoyé, puis oublie l'identifiant.
- download_stats() récupère une fois par jour stats.json (statistiques de tous les joueurs) depuis GitHub.
"""
import json
import time
import uuid
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from . import __version__
from .paths import USER_DIR
from .update import REPO

# Adresse de l'API (Cloudflare Workers). Vide = partage indisponible dans cette version.
API_URL = ""
STATS_URL = f"https://github.com/{REPO}/releases/download/stats/stats.json"

STATE_PATH = USER_DIR / "sharing.json"
QUEUE_PATH = USER_DIR / "share_queue.jsonl"
LAST_PATH = USER_DIR / "share_last.json"
STATS_PATH = USER_DIR / "stats.json"
MAX_AGE = 30 * 86400
MAX_GAMES, MAX_FEEDBACK = 20, 5
FEEDBACK_KINDS = ("idea", "bug", "other")


# ------------------------------------------------------------------ choix du joueur
def state():
    try:
        s = json.loads(STATE_PATH.read_text(encoding="utf-8"))
        return s if isinstance(s, dict) else {}
    except (OSError, ValueError):
        return {}


def _save(s):
    STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
    STATE_PATH.write_text(json.dumps(s), encoding="utf-8")


def asked():
    return state().get("enabled") is not None


def enabled():
    return bool(API_URL) and state().get("enabled") is True


def install_id():
    s = state()
    if not s.get("install"):
        s["install"] = str(uuid.uuid4())
        _save(s)
    return s["install"]


def set_enabled(on):
    s = state()
    s["enabled"] = bool(on)
    _save(s)
    if on:
        install_id()


# ------------------------------------------------------------------ résumé d'une partie
def _minion(m):
    """Entité du log ou photo de combat.py -> {"id", "a", "h", "g"} (None si la carte est cachée)."""
    if isinstance(m, dict):
        cid, a, h, g = m.get("card_id"), m.get("attack"), m.get("health"), m.get("golden")
    else:
        cid, a, h, g = m.card_id, m.get("ATK", 0), m.get("HEALTH", 0) - m.get("DAMAGE", 0), m.is_golden
    if not cid:
        return None
    return {"id": str(cid)[:40], "a": max(0, min(int(a or 0), 100000)), "h": max(0, min(int(h or 0), 100000)),
            "g": bool(g)}


def _board(items):
    return [m for m in map(_minion, items or []) if m][:7]


def game_summary(game, record):
    """record = history.game_record(...) de la même partie (héros, place, comp...)."""
    fights = []
    for f in getattr(game, "fights", [])[:40]:
        if f.get("result") not in ("W", "L", "T") or not f.get("turn"):
            continue
        fights.append({"t": max(1, min(int(f["turn"]), 60)), "tier": max(1, min(int(f.get("my_tier") or 1), 7)),
                       "otier": max(1, min(int(f.get("opp_tier") or 1), 7)),
                       "hp": max(-100, min(int(f.get("my_hp") or 0), 200)), "r": f["result"],
                       "d": max(0, min(int(f.get("damage") or 0), 100)),
                       "me": _board(f.get("my_board")), "opp": _board(f.get("board"))})
    # Pour repérer les parties abandonnées ou AFK (voir server/src/validate.js, gameQuality) : nombre
    # d'actions par tour (acheter, vendre, poser, relancer, monter, pouvoir héroïque... toutes comptent),
    # PV à la fin et abandon. Le serveur décide, l'appli ne juge pas.
    turns = max(1, min(int(getattr(game, "turn", 0) or 0), 60))
    acts = getattr(game, "actions_by_turn", {}) or {}
    actions = [max(0, min(int(acts.get(t, 0) or 0), 999)) for t in range(1, turns + 1)]
    return {"v": 1, "app": __version__, "mode": "duos" if record.get("duos") else "solo",
            "hero": record["hero"], "deity": record.get("deity"), "comp": record.get("comp"),
            "races": sorted(record.get("races") or [])[:6], "place": int(record["placement"]),
            "board": _board(game.my_board), "fights": fights, "turns": turns, "actions": actions,
            "end_hp": max(-100, min(int(getattr(game, "my_health", 0) or 0), 200)),
            "conceded": bool(getattr(game, "conceded", False))}


# ------------------------------------------------------------------ file d'attente locale
def _read_queue(now=None):
    now = time.time() if now is None else now
    try:
        lines = QUEUE_PATH.read_text(encoding="utf-8").splitlines()
    except OSError:
        return []
    items = []
    for line in lines:
        try:
            item = json.loads(line)
        except ValueError:
            continue
        if isinstance(item, dict) and now - item.get("queued", 0) < MAX_AGE:
            items.append(item)
    return items


def _write_queue(items):
    QUEUE_PATH.parent.mkdir(parents=True, exist_ok=True)
    QUEUE_PATH.write_text("".join(json.dumps(i) + "\n" for i in items), encoding="utf-8")


def enqueue(kind, data, now=None):
    items = _read_queue(now)
    items.append({"kind": kind, "data": data, "queued": time.time() if now is None else now})
    _write_queue(items[-200:])
    if kind == "game":
        LAST_PATH.write_text(json.dumps(data, indent=1), encoding="utf-8")


def add_game(game, record):
    """Fin de partie : met le résumé en file si le joueur partage. Renvoie True si ajouté."""
    if not enabled() or not record:
        return False
    enqueue("game", game_summary(game, record))
    return True


def add_feedback(kind, text):
    """Avis écrit dans la fenêtre "Send feedback" : envoyé avec le prochain envoi (même sans partage)."""
    text = (text or "").strip()[:1000]
    if kind not in FEEDBACK_KINDS or len(text) < 3:
        raise ValueError("feedback needs a kind and at least 3 characters")
    enqueue("feedback", {"kind": kind, "text": text, "app": __version__})


def pending():
    return _read_queue()


# ------------------------------------------------------------------ envois
def _request(method, url, body=None, timeout=20):
    if not url.startswith("https://"):
        raise ValueError("https only")
    data = None if body is None else json.dumps(body).encode("utf-8")
    req = Request(url, data=data, method=method,
                  headers={"content-type": "application/json", "User-Agent": f"Owlingstone/{__version__}"})
    try:
        with urlopen(req, timeout=timeout) as resp:  # nosec B310 : https vers notre API
            return resp.status
    except HTTPError as e:
        return e.code


def flush(send=_request):
    """Envoie la file en une requête. Renvoie le nombre d'éléments envoyés (0 si rien ou impossible)."""
    if not API_URL:
        return 0
    items = _read_queue()
    games = [i for i in items if i["kind"] == "game"][:MAX_GAMES] if enabled() else []
    notes = [i for i in items if i["kind"] == "feedback"][:MAX_FEEDBACK]
    if not games and not notes:
        return 0
    body = {"install": install_id(), "games": [g["data"] for g in games], "feedback": [n["data"] for n in notes]}
    try:
        status = send("POST", API_URL + "/v1/games", body)
    except OSError:
        return 0                                  # hors ligne : on réessaiera
    if status == 422:                             # refusé (version trop vieille ?) : on ne le renvoie pas
        status = 200
    if status != 200:
        return 0
    sent = {id(i) for i in games + notes}
    _write_queue([i for i in items if id(i) not in sent])
    return len(sent)


def delete_remote(send=_request):
    """Efface tout ce que cette installation a envoyé, puis oublie l'identifiant et vide la file."""
    s = state()
    if s.get("install") and API_URL:
        if send("DELETE", API_URL + "/v1/installs/" + s["install"]) != 200:
            return False
    s.pop("install", None)
    s["enabled"] = False
    _save(s)
    _write_queue([i for i in _read_queue() if i["kind"] != "game"])
    for p in (LAST_PATH,):
        try:
            p.unlink()
        except OSError:
            pass
    return True


def download_stats(now=None, timeout=20):
    """stats.json de tous les joueurs (GitHub), au plus une fois par jour. Renvoie le dict ou None."""
    now = time.time() if now is None else now
    try:
        if now - STATS_PATH.stat().st_mtime < 86400:
            return json.loads(STATS_PATH.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        pass
    try:
        req = Request(STATS_URL, headers={"User-Agent": f"Owlingstone/{__version__}"})
        with urlopen(req, timeout=timeout) as resp:  # nosec B310 : URL fixe https (GitHub)
            data = resp.read(2_000_001)
        if len(data) > 2_000_000:
            return None
        stats = json.loads(data)
        STATS_PATH.write_bytes(data)
        return stats if isinstance(stats, dict) else None
    except (OSError, ValueError):
        return None
