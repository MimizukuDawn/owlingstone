# Privacy

Owlingstone works offline. It reads Hearthstone's log file on your PC and keeps everything it learns
(game history, settings, card images, learned data) in `%APPDATA%\Owlingstone`.

It contacts only:

- **GitHub** (`api.github.com`, `github.com`) to check for and download new versions;
- **HearthstoneJSON** (`api.hearthstonejson.com`, `art.hearthstonejson.com`) for card data and art.

## Share my games (coming soon, opt-in)

A later version will offer to share anonymous game summaries to improve everyone's advice. It will be
off unless you say yes, and this file will list exactly what is sent before it ships. It will never send
your BattleTag, other players' names, your log files or file paths, and you will be able to delete
everything you shared.
