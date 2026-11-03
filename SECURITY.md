# Security

Please report a security problem privately with GitHub's **Report a vulnerability** button
(Security tab of this repository), not in a public issue.

How this project protects its users:

- Releases are built by GitHub Actions with a build provenance attestation; the build job has read-only
  rights and every Rust crate is pinned in `Cargo.lock` (builds use `--locked`).
- Each release is then signed (Ed25519) with the maintainer's offline key, which is never on GitHub. The app
  installs an update only if that signature matches the public key built into it and the exe's SHA-256 is
  the signed one, so a stolen GitHub account or a poisoned build could not push an update to players.
  It keeps the old version if anything fails.
- No key or secret is stored in the code or the app. Every push runs the tests, a privacy and secret scan
  (`tools/privacy_scan.py`) and `cargo clippy` with warnings as errors.
- Data files are parsed as plain data (serde); no code is ever built from game logs or downloaded data.
