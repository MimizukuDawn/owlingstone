"""Récupère les avis des joueurs (pour le mainteneur uniquement : il faut la clé admin).

    python tools/feedback.py fetch        -> ajoute les nouveaux avis à feedback/inbox.jsonl
    python tools/feedback.py done 12 13   -> marque des avis comme traités sur le serveur

La clé admin vient de la variable OWLINGSTONE_ADMIN_KEY ou du fichier admin_key.txt dans le dossier de
données (%APPDATA%\\Owlingstone). Elle n'est jamais dans le dépôt. Le dossier feedback/ est ignoré par git :
les avis ne sont jamais publiés. Ensuite, Claude les regroupe (idées proches ensemble), repère les
propositions contraires et prépare la liste des décisions à prendre et des mises à jour possibles.
"""
import json
import os
import sys
from pathlib import Path
from urllib.request import Request, urlopen

# Same values as the app (src/paths.rs, src/share.rs).
API_URL = "https://owlingstone-api.mimizuku-dawn.workers.dev"
USER_DIR = Path(os.environ.get("OWLINGSTONE_HOME") or Path(os.environ.get("APPDATA") or Path.home()) / "Owlingstone")

INBOX = Path(__file__).resolve().parent.parent / "feedback" / "inbox.jsonl"


def admin_key():
    key = os.environ.get("OWLINGSTONE_ADMIN_KEY")
    if not key:
        try:
            key = (USER_DIR / "admin_key.txt").read_text(encoding="utf-8").strip()
        except OSError:
            sys.exit("No admin key: set OWLINGSTONE_ADMIN_KEY or create admin_key.txt in " + str(USER_DIR))
    return key


def call(method, path, body=None):
    if not API_URL.startswith("https://"):
        sys.exit("API_URL is not set")
    req = Request(API_URL + path, method=method, data=None if body is None else json.dumps(body).encode(),
                  headers={"authorization": "Bearer " + admin_key(), "content-type": "application/json",
                           "User-Agent": "Owlingstone-maintainer"})
    with urlopen(req, timeout=30) as resp:  # nosec B310 : https vers notre API
        return json.loads(resp.read())


def fetch():
    known = set()
    if INBOX.exists():
        known = {json.loads(line)["id"] for line in INBOX.read_text(encoding="utf-8").splitlines() if line.strip()}
    new = [f for f in call("GET", f"/v1/admin/feedback?after={max(known, default=0)}")["feedback"]
           if f["id"] not in known]
    INBOX.parent.mkdir(exist_ok=True)
    with open(INBOX, "a", encoding="utf-8") as out:
        for f in new:
            out.write(json.dumps(f, ensure_ascii=False) + "\n")
    print(f"{len(new)} new feedback -> {INBOX}")


def main(argv):
    if argv[:1] == ["fetch"]:
        fetch()
    elif argv[:1] == ["done"] and argv[1:]:
        print(call("POST", "/v1/admin/feedback/done", {"ids": [int(i) for i in argv[1:]]}))
    else:
        print(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
