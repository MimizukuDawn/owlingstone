# Owlingstone

A free, standalone helper for **Hearthstone Battlegrounds** on Windows. It reads the game's own log file
(`Power.log`) and shows, next to the game:

- the minion tier list for the tribes in your lobby, with what is in your shop, board and hand;
- hero pick advice and the comps that work in this lobby, with what to buy for them;
- a turn plan: level up or not, what to buy, sell, roll or freeze, with the danger of the next fight;
- your chance to win each fight (a combat simulator), and the lobby: every player's last board;
- your own game history and stats.

It does not read the game's memory, inject anything or automate any action: it only reads the log file
Hearthstone writes on your PC, like other community trackers.

## Install

1. Download **`Owlingstone.exe`** from the [latest release](https://github.com/owlingstone/owlingstone/releases/latest).
2. Put it anywhere you can write to (for example your Documents folder or Desktop) and run it. That's all.
3. Windows may say "Windows protected your PC" because the app is not signed with a paid certificate:
   click **More info**, then **Run anyway**.

Updates: when a new version is published, the app downloads it in the background, checks that it carries
the maintainer's signature (made with a key that is never on GitHub) and its SHA-256 checksum, and shows
**Update to vX**: one click replaces the exe and restarts it, with no window to deal
with. Your history and settings (in `%APPDATA%\Owlingstone`) are kept.

### Check your download (optional)

The release notes give the exe's SHA-256 (under "Check the download"), and each release has a GitHub
build attestation proving the exe was built by GitHub Actions from this repository's code:

```powershell
Get-FileHash .\Owlingstone.exe -Algorithm SHA256   # compare with the SHA-256 in the release notes
gh attestation verify .\Owlingstone.exe --repo owlingstone/owlingstone
```

### Hearthstone logs

The app reads `Power.log` in the game's `Logs` folder. It finds the folder by itself, wherever Hearthstone
is installed (from the running game or Battle.net's install record). If it can't, right-click the tray icon
> **Choose Hearthstone folder…**. If Hearthstone's logging is off (you never used a deck tracker), the app
turns it on; restart the game once if it was already open.

## Privacy

Nothing about your games leaves your PC unless you turn on **Share my games** (off by default, asked
once). Your history, settings and learned data stay in `%APPDATA%\Owlingstone`. See [PRIVACY.md](PRIVACY.md)
for exactly what is shared when you opt in and how to delete it.

## Feedback

Right-click the Owlingstone icon near the clock > **Send feedback…** to send an idea or a bug report.

## Build from source

Install [Rust](https://rustup.rs/), then:

```powershell
cargo run --release              # the overlay
cargo test                       # the tests
cargo run --release -- history   # your stats in the console
```

The exe is `target\release\Owlingstone.exe`. The comps, heroes, hero curves, economy and rules are plain
YAML files in `data/`, built into the exe; when the exe runs from the project (a `data/` folder above it),
it reads that folder instead and reloads a file when you edit it.

## Data sources and credits

- Card data and art: [HearthstoneJSON](https://hearthstonejson.com/), downloaded at runtime (a copy of the
  card data ships in `data/cards-fallback.json.gz` so the app also starts offline).
- Comp, hero and trinket statistics in `data/`: community data from Firestone, as published by
  [bgbuddy.gg](https://bgbuddy.gg/).
- The combat simulator is original code written from the game's rules.

Hearthstone and Battlegrounds are trademarks of Blizzard Entertainment. This project is not affiliated
with or endorsed by Blizzard Entertainment.

## License

[PolyForm Noncommercial 1.0.0](LICENSE.md): free to use, change and share for any noncommercial purpose.
Selling it or using it in a commercial product is not allowed.
