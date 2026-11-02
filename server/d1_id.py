"""Prints the id of the "owlingstone" D1 database (creates it if --create and missing).
Used by the workflows. Actions logs are public, so on failure it prints only wrangler's error lines,
with ids, e-mail addresses and account names hidden (wrangler's own output names the account owner).

    python3 d1_id.py --filter < out.txt     same safe filter for any wrangler output"""
import json
import re
import subprocess  # nosec B404 : runs the wrangler CLI with fixed arguments
import sys

WRANGLER = ["npx", "--yes", "wrangler@4.156.2"]


def masked(text):
    """Keeps only error lines; hides ids, e-mails and anything naming the account."""
    keep = []
    for line in text.splitlines():
        if not re.search(r"ERROR|\[code: \d+\]|error:", line, re.I) or re.search(r"account name|'s account", line, re.I):
            continue
        line = re.sub(r"[\w.+-]+@[\w-]+(\.[\w-]+)+", "<hidden>", line)
        line = re.sub(r"[0-9a-fA-F-]{16,}", "<hidden>", line)
        line = re.sub(r"https?://\S+", "<url>", line)
        keep.append(line.strip()[:200])
    return "\n".join(keep[-10:]) or "(no error line; full output hidden because logs are public)"


def run(*args):
    p = subprocess.run(WRANGLER + list(args), capture_output=True, text=True)  # nosec B603
    return p.returncode, p.stdout, p.stderr


def find():
    code, out, err = run("d1", "list", "--json")
    start = out.find("[")
    try:
        dbs = json.loads(out[start:]) if start >= 0 else None
    except ValueError:
        dbs = None
    if code != 0 or dbs is None:
        sys.stderr.write("wrangler d1 list failed:\n" + masked(out + err)[-1500:] + "\n")
        sys.exit(1)
    return next((d["uuid"] for d in dbs if d.get("name") == "owlingstone"), "")


def main(argv):
    if "--filter" in argv:
        sys.stderr.write(masked(sys.stdin.read()) + "\n")
        sys.exit(1)
    db = find()
    if not db and "--create" in argv:
        code, out, err = run("d1", "create", "owlingstone")
        if code != 0:
            sys.stderr.write("wrangler d1 create failed:\n" + masked(out + err)[-1500:] + "\n")
            sys.exit(1)
        db = find()
    print(db)


if __name__ == "__main__":
    main(sys.argv[1:])
