"""Prints the id of the "owlingstone" D1 database (creates it if --create and missing).
Used by the workflows. On failure it prints wrangler's error with every long hex id masked, because
Actions logs are public."""
import json
import re
import subprocess  # nosec B404 : runs the wrangler CLI with fixed arguments
import sys

WRANGLER = ["npx", "--yes", "wrangler@4.156.2"]


def masked(text):
    return re.sub(r"[0-9a-f]{16,}|[0-9a-f-]{36}", "<hidden>", text)


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
