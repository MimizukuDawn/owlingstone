# Privacy

Owlingstone works offline. It reads Hearthstone's log file on your PC and keeps everything it learns
(game history, settings, card images, learned data) in `%APPDATA%\Owlingstone`.

It contacts only:

- **GitHub** (`api.github.com`, `github.com`) to check for and download new versions, and once a day the
  shared stats file;
- **HearthstoneJSON** (`api.hearthstonejson.com`, `art.hearthstonejson.com`) for card data and art;
- **the Owlingstone server** (a Cloudflare Worker) only if you turn on **Share my games** or send feedback.

## Share my games (optional, off unless you say yes)

The app asks once. If you say yes, at the end of each finished game it sends one summary:

| Sent | Never sent |
| --- | --- |
| A random install id, made on your PC when you turn sharing on | Your BattleTag, account, or other players' names |
| App version, Solo or Duos | Your log files, file paths, PC name |
| Hero, Deity, lobby tribes, final placement, detected comp | Date and time of the game |
| Final board: card ids, attack, health, golden | Your settings or history file |
| Each fight: turn, both tavern tiers, your health, result, damage, both boards (card ids and stats) | Games played before you turned sharing on |
| Number of actions you made each turn, your health at the end, whether you conceded | Which actions, or when |

The server stores the day it received the summary, not your IP address. Summaries are grouped (up to 20
games per request) and sent at the end of a game or when the app starts.

In the tray menu (right-click the icon):

- **Share my games**: turn it on or off at any time.
- **Show what is sent**: the exact last summary, as sent.
- **Delete my shared games**: deletes everything sent with your install id from the server and forgets it.

Games where a player was AFK (3 turns in a row with no action at all) or conceded while still healthy are
kept out of the shared stats; early concedes are only counted by likely reason (lost streak, behind on
tier, weak board...), never shown one by one. Shared games are kept 60 days. Only totals (average placement per comp and hero, typical boards per
turn, with at least 30 games each) are published, in the `stats` release of this repository.

## Feedback

Right-click the tray icon > **Send feedback…** sends your text (1000 characters at most) with the same
random install id. Feedback is never published; only the maintainer reads it, to plan updates. Please don't
write personal information in it. **Delete my shared games** deletes it too.
