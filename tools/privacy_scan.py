"""Vérifie qu'un dossier prêt à publier ne contient aucune donnée personnelle ni secret.

    python tools/privacy_scan.py [dossier] [--terms fichier]

Cherche dans chaque fichier (texte et binaire) :
- des fichiers interdits (historique, réglages, logs du jeu, subsets.json, NOTES.md, .env...) ;
- des BattleTags (Nom#1234), des adresses e-mail, des chemins Windows/Mac/Linux avec un nom d'utilisateur ;
- des secrets : clés privées, jetons GitHub / Cloudflare / Supabase, longues chaînes aléatoires ;
- des liens vers des documents privés (claude.ai) ;
- les mots personnels du fichier --terms (un par ligne, jamais publié : par défaut
  %APPDATA%\\Owlingstone\\private_terms.txt), par exemple son pseudo ou son nom.
Code de sortie 1 s'il trouve quelque chose : la publication doit s'arrêter.
"""
import math
import os
import re
import sys
from pathlib import Path

FORBIDDEN_NAMES = {"subsets.json", "history.jsonl", "settings.json", "cards.json", "error.log",
                   "NOTES.md", ".env", "private_terms.txt", "id_rsa", "id_ed25519"}
FORBIDDEN_DIRS = {"testdata", "shots", "ref", "build", "dist", ".venv", "tiles", "__pycache__", ".pytest_cache"}
FORBIDDEN_SUFFIXES = {".log", ".jsonl", ".pem", ".key", ".p12", ".pfx", ".sqlite", ".db"}

PATTERNS = [
    ("BattleTag", re.compile(r"(?<![\w#&])[A-Za-zÀ-ɏ][\wÀ-ɏ]{1,15}#\d{3,6}\b")),
    ("e-mail", re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")),
    ("user path", re.compile(r"(?i)(?:[A-Z]:\\\\?Users\\\\?|/Users/|/home/)(?!runner\b|<|%|\{|\$)[\w.-]+")),
    ("private key", re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")),
    ("GitHub token", re.compile(r"\b(?:ghp|gho|ghu|ghs|ghr|github_pat)_[A-Za-z0-9_]{20,}")),
    ("JWT / Supabase key", re.compile(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}")),
    ("API key assignment", re.compile(r"(?i)(?:api[_-]?key|api[_-]?token|secret|password|passwd|auth)"
                                      r"\s*[:=]\s*[\"']?[A-Za-z0-9_\-/+=]{16,}")),
    ("private link", re.compile(r"https?://(?:www\.)?claude\.ai/\S+")),
    ("account id (32 hex)", re.compile(r"(?<![0-9a-fA-F])[0-9a-f]{32}(?![0-9a-fA-F])")),
    ("dashboard link", re.compile(r"https?://dash\.cloudflare\.com/\S+")),
]
_RANDOM = re.compile(r"[A-Za-z0-9_\-+/=]{32,}")
_BINARY_ALLOWED = {".ico", ".png"}


def entropy(s):
    counts = {c: s.count(c) for c in set(s)}
    return -sum(n / len(s) * math.log2(n / len(s)) for n in counts.values())


def default_terms_file():
    base = os.environ.get("APPDATA")
    return Path(base) / "Owlingstone" / "private_terms.txt" if base else Path.home() / ".owlingstone" / "private_terms.txt"


def load_terms(path):
    try:
        lines = Path(path).read_text(encoding="utf-8").splitlines()
    except OSError:
        return []
    return [t.strip() for t in lines if t.strip() and not t.startswith("#")]


def scan(root, terms=()):
    """Liste de (fichier, ligne, type, extrait). Vide = rien à signaler."""
    root = Path(root)
    found = []
    term_res = [(t, re.compile(r"(?<!\w)" + re.escape(t) + r"(?!\w)", re.I if t.islower() else 0)) for t in terms]
    for path in sorted(root.rglob("*")):
        if path.is_dir() or ".git" in path.relative_to(root).parts:
            continue
        rel = path.relative_to(root).as_posix()
        parts = set(path.relative_to(root).parts[:-1])
        if path.name in FORBIDDEN_NAMES or parts & FORBIDDEN_DIRS or path.suffix.lower() in FORBIDDEN_SUFFIXES:
            found.append((rel, 0, "forbidden file", path.name))
            continue
        data = path.read_bytes()
        text = data.decode("utf-8", errors="replace")
        if b"\0" in data[:4096] and path.suffix.lower() not in _BINARY_ALLOWED:
            found.append((rel, 0, "unexpected binary", f"{len(data)} bytes"))
        for no, line in enumerate(text.splitlines(), 1):
            for kind, rx in PATTERNS:
                for m in rx.finditer(line):
                    if kind == "BattleTag" and m.group(0).endswith("#1234"):
                        continue                     # exemple inventé (Player#1234)
                    found.append((rel, no, kind, m.group(0)[:80]))
            for term, rx in term_res:
                if rx.search(line):
                    found.append((rel, no, "personal term", term))
            for m in _RANDOM.finditer(line):
                s = m.group(0)
                if entropy(s) > 4.3 and any(c.isdigit() for c in s) and any(c.isupper() for c in s):
                    found.append((rel, no, "random-looking string", s[:40]))
    return found


def main(argv):
    args = list(argv)
    terms_file = default_terms_file()
    if "--terms" in args:
        i = args.index("--terms")
        terms_file = Path(args[i + 1])
        del args[i:i + 2]
    root = Path(args[0]) if args else Path(__file__).resolve().parent.parent
    terms = load_terms(terms_file)
    found = scan(root, terms)
    files = sum(1 for p in root.rglob("*") if p.is_file() and ".git" not in p.relative_to(root).parts)
    print(f"Scanned {files} files in {root} ({len(terms)} personal terms from {terms_file})")
    for rel, no, kind, text in found:
        print(f"  {rel}:{no}  [{kind}]  {text}")
    print("CLEAN: nothing personal or secret found." if not found else f"FOUND {len(found)} problem(s).")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
