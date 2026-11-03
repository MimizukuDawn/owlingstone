"""The release signing key: make it once, then sign each release with it. Runs on the maintainer's PC only.

    python tools/release_key.py new  --key <file>                 make the secret key (refuses to overwrite)
    python tools/release_key.py pub  --key <file>                 print its public key (goes in src/update.rs)
    python tools/release_key.py sign --key <file> --version 1.9.0 (--sha256 <hex> | --exe Owlingstone.exe)
                                                                   write signatures/v1.9.0.sig
    python tools/release_key.py verify --sig signatures/v1.9.0.sig --version 1.9.0
                                                                   check it with the app's public keys

Why: every installed Owlingstone checks an update's signature with the public key built into it
(PUBLIC_KEYS in src/update.rs) before replacing itself. The secret key never goes to GitHub, so even someone who
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
sys.path.insert(0, str(Path(__file__).resolve().parent))
import ed25519  # noqa: E402

HEADER = "owlingstone-release-key 1"


def read_key(path):
    lines = Path(path).read_text(encoding="utf-8").split()
    if len(lines) != 4 or " ".join(lines[:2]) != HEADER or lines[2] != "seed" or not re.fullmatch(r"[0-9a-f]{64}", lines[3]):
        raise SystemExit(f"{path} is not an Owlingstone release key")
    return bytes.fromhex(lines[3])


def signed_text(version, sha):
    return f"owlingstone-release 1\nversion {version}\nsha256 {sha}\n"


def app_public_keys():
    """The public keys built into the app (PUBLIC_KEYS in src/update.rs)."""
    src = (ROOT / "src" / "update.rs").read_text(encoding="utf-8")
    m = re.search(r"PUBLIC_KEYS: &\[&str\] = &\[([^\]]*)\]", src)
    return [bytes.fromhex(k) for k in re.findall(r'"([0-9a-f]{64})"', m.group(1) if m else "")]


def verify(sig_text, version):
    """The SHA-256 a signature file signs for this version; SystemExit if it is not valid."""
    lines = sig_text.split("\n")
    m = re.fullmatch(r"signature ([0-9a-f]{128})", lines[3] if len(lines) > 3 else "")
    sha = lines[2][len("sha256 "):] if len(lines) > 2 else ""
    text = signed_text(version, sha)
    if not m or "\n".join(lines[:3]) + "\n" != text or not re.fullmatch(r"[0-9a-f]{64}", sha):
        raise SystemExit(f"not a signature for version {version}")
    if not any(ed25519.verify(k, text.encode("ascii"), bytes.fromhex(m.group(1))) for k in app_public_keys()):
        raise SystemExit("the signature does not match the app's public key")
    return sha


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("action", choices=["new", "pub", "sign", "verify"])
    ap.add_argument("--key")
    ap.add_argument("--sig")
    ap.add_argument("--version")
    ap.add_argument("--sha256")
    ap.add_argument("--exe")
    ap.add_argument("--out-dir", default=str(ROOT / "signatures"))
    a = ap.parse_args(argv)
    if a.action == "verify":
        if not a.sig or not a.version:
            raise SystemExit("--sig <file> and --version are required")
        print(verify(Path(a.sig).read_text(encoding="ascii"), a.version))
        return 0
    if not a.key:
        raise SystemExit("--key <file> is required")
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
