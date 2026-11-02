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

1. Download **`Owlingstone.exe`** from the [latest release](https://github.com/MimizukuDawn/owlingstone/releases/latest).
2. Put it anywhere you can write to (for example your Documents folder or Desktop) and run it. That's all.
3. Windows may say "Windows protected your PC" because the app is not signed with a paid certificate:
   click **More info**, then **Run anyway**.

Updates: when a new version is published, the app downloads it in the background, checks its SHA-256
checksum and shows **Update to vX**: one click replaces the exe and restarts it, with no window to deal
with. Your history and settings (in `%APPDATA%\Owlingstone`) are kept.

The release also has `Owlingstone-<version>-windows.zip`: the same app as a folder, which starts a little faster.

### Check your download (optional)

Each release has `.sha256` files and a GitHub build attestation proving the files were built by GitHub
Actions from this repository's code:

```powershell
Get-FileHash .\Owlingstone.exe -Algorithm SHA256   # compare with Owlingstone.exe.sha256
gh attestation verify .\Owlingstone.exe --repo MimizukuDawn/owlingstone
```

### Hearthstone logs

The app reads `Power.log` in the game's `Logs` folder (default `C:\Program Files (x86)\Hearthstone\Logs`).
If you have never used a deck tracker, Hearthstone may not write that log yet: start the game once with a
tracker such as Hearthstone Deck Tracker or Firestone, which turns logging on.

## Privacy

Nothing about your games leaves your PC unless you turn on **Share my games** (off by default, asked
once). Your history, settings and learned data stay in `%APPDATA%\Owlingstone`. See [PRIVACY.md](PRIVACY.md)
for exactly what is shared when you opt in and how to delete it.

## Feedback

Right-click the Owlingstone icon near the clock > **Send feedback…** to send an idea or a bug report.

## Run from source

```powershell
py -3.12 -m venv .venv
.venv\Scripts\python -m pip install -r requirements.txt
.venv\Scripts\python -m bgtracker          # the overlay
.venv\Scripts\python -m pytest -q -m "not slow"
```

`update.bat` tests the code, builds `dist\Owlingstone\Owlingstone.exe` and starts it. The comps, heroes, hero
curves, economy and rules are plain YAML files in `data/`: edit them and the app reloads them.

## Data sources and credits

- Card data and art: [HearthstoneJSON](https://hearthstonejson.com/), downloaded at runtime.
- Comp, hero and trinket statistics in `data/`: community data from Firestone, as published by
  [bgbuddy.gg](https://bgbuddy.gg/).
- The combat simulator is original code written from the game's rules.

Hearthstone and Battlegrounds are trademarks of Blizzard Entertainment. This project is not affiliated
with or endorsed by Blizzard Entertainment.

## License

[PolyForm Noncommercial 1.0.0](LICENSE.md): free to use, change and share for any noncommercial purpose.
Selling it or using it in a commercial product is not allowed.
