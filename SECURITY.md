# Security

Please report a security problem privately with GitHub's **Report a vulnerability** button
(Security tab of this repository), not in a public issue.

How this project protects its users:

- Releases are built only by GitHub Actions from a tagged commit, with a SHA-256 checksum and a build
  provenance attestation.
- The app only downloads updates from this repository's Releases over HTTPS, refuses a download whose
  checksum does not match, and keeps the old version if anything fails.
- No key or secret is stored in the code or the app. Every push runs the tests, a privacy and secret scan
  (`tools/privacy_scan.py`) and `bandit`.
- Data files are parsed with `yaml.safe_load`; no code is ever built from game logs or downloaded data.
