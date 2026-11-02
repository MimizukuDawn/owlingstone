"""The release signing key: make it once, then sign each release with it. Runs on the maintainer's PC only.

    python tools/release_key.py new  --key <file>                 make the secret key (refuses to overwrite)
    python tools/release_key.py pub  --key <file>                 print its public key (goes in update.py)
    python tools/release_key.py sign --key <file> --version 1.9.0 (--sha256 <hex> | --exe Owlingstone.exe)
                                                                   write signatures/v1.9.0.sig

Why: every installed Owlingstone checks an update's signature with the public key built into it
(update.PUBLIC_KEYS) before replacing itself. The secret key never goes to GitHub, so even someone who
took over the GitHub account or the build could not ship an update players would accept.

The secret key file is 32 random bytes written as hex. Keep a copy offline (a USB stick): without it,
no further update can be signed, and players would have to download a new version by hand.

A signature is a small text file. The signed text is its first three lines:
    owlingstone-release 1
    version 1.9.0
    sha256 <SHA-256 of Owlingstone.exe>
    signature <Ed25519 signature of the three lines above, hex>
Pushing signatures/v<version>.sig to main attaches it to the Release (see .github/workflows/sign.yml).
"""
import argparse
import hashlib
import re
import secrets
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from bgtracker import ed25519  # noqa: E402

HEADER = "owlingstone-release-key 1"


def read_key(path):
    lines = Path(path).read_text(encoding="utf-8").split()
    if len(lines) != 4 or " ".join(lines[:2]) != HEADER or lines[2] != "seed" or not re.fullmatch(r"[0-9a-f]{64}", lines[3]):
        raise SystemExit(f"{path} is not an Owlingstone release key")
    return bytes.fromhex(lines[3])


def signed_text(version, sha):
    return f"owlingstone-release 1\nversion {version}\nsha256 {sha}\n"


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("action", choices=["new", "pub", "sign"])
    ap.add_argument("--key", required=True)
    ap.add_argument("--version")
    ap.add_argument("--sha256")
    ap.add_argument("--exe")
    ap.add_argument("--out-dir", default=str(ROOT / "signatures"))
    a = ap.parse_args(argv)
    key = Path(a.key)
    if a.action == "new":
        if key.exists():
            raise SystemExit(f"{key} already exists: refusing to replace a signing key")
        key.parent.mkdir(parents=True, exist_ok=True)
        key.write_text(f"{HEADER}\nseed {secrets.token_bytes(32).hex()}\n", encoding="utf-8")
        print("public key:", ed25519.public_key(read_key(key)).hex())
        return 0
    seed = read_key(key)
    if a.action == "pub":
        print(ed25519.public_key(seed).hex())
        return 0
    if not a.version or not re.fullmatch(r"\d+(\.\d+){1,3}", a.version):
        raise SystemExit("--version 1.2.3 is required")
    sha = (a.sha256 or "").lower()
    if a.exe:
        sha = hashlib.sha256(Path(a.exe).read_bytes()).hexdigest()
    if not re.fullmatch(r"[0-9a-f]{64}", sha):
        raise SystemExit("--sha256 <64 hex> or --exe <file> is required")
    text = signed_text(a.version, sha)
    sig = ed25519.sign(seed, text.encode("ascii"))
    out = Path(a.out_dir) / f"v{a.version}.sig"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text + f"signature {sig.hex()}\n", encoding="ascii", newline="\n")
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
