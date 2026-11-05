# Owlingstone – what's new (formerly BG Tracker)

The app shows the lines of its version after an update. Newest version first.

## 2.2.3
- Fixed: "Quit Owlingstone" in the tray menu now really closes the app. Before, the windows went away but Owlingstone could keep running in the background (still in the Task Manager, and a new start did nothing). The app now saves your settings, removes its tray icon and ends at once. Updating uses the same path, so the new version starts right away.

## 2.2.2
- Choice odds: the strip now sits just under the win chance banner instead of over it, and stays in the middle of the board (narrower columns) so it no longer covers the hero portraits on the left.

## 2.2.1
- Fixed: the win chance shown before a fight was almost always 0% or 100%, and often wrong (on 190 recent fights, "100%" fights were really won 63% of the time and "0%" fights 20%). The opponent's board is a guess at that point, and the app simulated a single guess. It now simulates the guess at 7 strengths, from much weaker to much stronger (the real board is often half or twice as strong), and averages them. On the same fights, the odds now match what happened: "about 20%" wins about 24% of the time, "about 80%" about 88%. The turn advice and the odds on choices use the same, more honest odds.
- The odds during the fight (both boards known) were already right; they are unchanged.
- Fixed: fights against a dead player's ghost, and a few other fights where the log shows no damage, were counted as ties. The minions left now decide the result, so the game's "expected wins / real wins" line and your history are right.

## 2.2.0
- New: odds on the game's choices. When Hearthstone asks you to pick (your hero at the start, a Trinket offer, or any Discover: Triple reward, Tavern spells, hero powers and minions that Discover), a strip at the top of the game shows, over each option, its chance to be the best pick, marks the one to take, and says why in a few words.
- How it guesses: heroes and Trinkets by their expected place (the same stats as the side panel); minions and spells by what they bring you now: a card of the comp you play, a triple or a pair, stats, gold or extra cards, synergy with your board, and the change in your next fight (simulated against your next opponent's expected board). Options with no data share the odds evenly and the strip says so.
- Click the × to hide it for that choice, or turn it off in the tray menu ("Odds on choices"). The columns are placed from screenshots: if they sit off the cards on your screen, tell us with "Send feedback".

## 2.1.6
- Up to date with Hearthstone patch 37.0.2 (November 3): new card stats and texts, Expert Aviator out of the pool, Murcules back, Harbinger Aph'lass now Tier 5 and Hot-Air Surveyor Tier 5.
- Trinkets: the new prices are used in the advice, Weighted Gauntlet is now a Greater Trinket, and Spell-powered Wrench, Quilligraphy Set (Greater), Fridge Magnet and Writhing Tentacles are no longer offered. The trinket and comp stats are still from the last patch until there are enough games on the new one.
- Comps: Murloc Scam now runs on Diremuck Forager instead of Expert Aviator; Murcules joins Murloc Hand Buff.
- Win chance: Ship Master Eudora gives +8/+8, Stalwart Kodo gives double its stats (triple when golden), The Last One Standing gives +10/+10 and grows after each Rally, Sanguine Refiner's Blood Gem bonus is +1/+1.

## 2.1.5
- Fixed: the card preview, the opponent board on portrait hover, the win chance banner and the "running" badge no longer show as an empty grey box. Windows doesn't draw click-through windows made this way, so these windows no longer let clicks through (none of them sits over anything you click).
- The opponent board now opens at the right of the game, past the game's own portrait tooltip (hero powers and trinkets included) instead of under it, with 4 cards per row.

## 2.1.4
- Less lag when you level up, sell or buy: the fight simulations behind the turn plan and the win chance now run on half of your CPU at most, and Owlingstone's background work always lets Hearthstone go first. The advice may show a split second later on small PCs; the game stays smooth.
- Fixed the win chance banner at the top of the game sometimes showing as an empty box.

## 2.1.3
- In Hearthstone's menus, a small gold badge ("Owlingstone · running, waiting for a game") now shows where the left panel goes, so you can see the app is on. After a game it shows where you finished. The panels themselves still appear only during a game (with auto-hide on).

## 2.1.1
- The tray menu closes as soon as you click anywhere else (the game, the desktop, the taskbar), instead of staying open for several seconds.
- New in the tray menu: **Check for updates now**. It asks GitHub right away instead of waiting for the check every 6 hours, and says if you already have the latest version.

## 2.1.0
- Put your mouse on a hero portrait in the game's leaderboard (left of the board): Owlingstone shows that player's last board you fought, with the card pictures, their attack and health in that fight, golden minions, the turn and who won. Players you haven't fought yet say so.
- Pictures are downloaded once and kept in your data folder, like the card previews. Turn it off in the tray menu ("Opponent board on portrait hover"). Not shown in Duos for now.

## 2.0.2
- The panels now appear and vanish at once when you switch to or away from Hearthstone: no more slow fade, and no more wait of up to half a second before they react.
- The "My stats" button no longer gets cut off at the bottom of the left panel: it moved next to the version number.

## 2.0.1
- The tray menu now has the overlay's look: dark with gold, instead of the plain white Windows menu. Left or right click the owl icon to open it; click outside, press Escape or click the icon again to close it.

## 2.0.0
- Rewritten from scratch in Rust: the download is smaller (about 17 MB instead of 27 MB), and the app opens faster and is lighter on your PC while you play.
- Everything you had is still there: tier list, Comps and Lobby tabs, turn plan, win chance banner, card previews, hero and trinket advice, history and stats, sharing and feedback. Your settings, history and learned data carry over untouched.
- The combat simulator is much faster, so the win chance is ready sooner after each fight starts.
- Duos: the app now checks which player is you before showing anything, instead of guessing.
- Shared stats: one very active player can no longer tilt the community numbers (each install counts for at most 10 games per comp or hero).
- Opens even without a working OpenGL driver (virtual machines, remote desktop, old drivers): it then draws with Direct3D instead.
- Commands for the curious: run `Owlingstone.exe history` in a console to see your stats there.

## 1.9.1
- Fixed: after starting the app (or after an update), the overlay no longer replays the evening's old games one by one in the menu. It now jumps straight to the last game in Hearthstone's log and shows nothing until it has caught up.
- Once a game is over you are back in the menus: the overlay drops the turn, the combat odds and the turn plan, and only says where you finished (with auto-hide on, it hides until the next game).
- A game already recorded is not sent again when the app restarts.

## 1.9.0
- Works wherever Hearthstone is installed: the app finds the Logs folder from the running game or Battle.net's install record, and you can pick it yourself (tray menu > Choose Hearthstone folder…). If Hearthstone's logging is off, the app turns it on. Instead of "Waiting for a Battlegrounds game" forever, it says what is missing.
- Starts without internet: card data is now built into the app, and it refreshes in the background every week and as soon as a game shows a card it doesn't know (new patch), so new heroes and minions appear without waiting.
- Signed updates: every new version must carry the maintainer's signature, made with a key that never touches GitHub. Updates without it are refused, even if the GitHub account were stolen.
- The overlay no longer freezes after an unexpected log line or error: the error goes to error.log (in your data folder) and reading carries on.
- Feedback typed while games are being sent is never lost, and one bad game no longer makes the server refuse the others.
- The sharing question now comes after your first finished game, not before you've seen the app.
- Lighter on your PC: the log folder is checked every 5 seconds instead of 4 times a second, the log is read in small pieces, and only the current game stays in memory.
- The project moved to github.com/owlingstone/owlingstone.

## 1.8.4
- Releases now hold just **Owlingstone.exe**: no zip, no checksum files to puzzle over. The app still checks every update's SHA-256 (GitHub records it for each file), and the release notes show it for anyone who wants to check by hand.
- About half the size: the parts of Qt the app never uses (QML, PDF, software OpenGL, virtual keyboard, translations...) are left out, so updates download faster and the exe opens faster. Every build is started once on GitHub before release to prove it still opens.
- Faster rebuilds from the project: packages are reinstalled only when requirements.txt changes, and PyInstaller reuses what didn't change.

## 1.8.3
- One file to download: each release now has **Owlingstone.exe**, the whole app in a single exe. Put it anywhere and run it. It updates itself in place (the new exe is checked with its SHA-256 before it replaces the old one). The zip is still there for those who prefer the folder version.
- Updates run without a cmd window: click "Update to vX" and Owlingstone restarts by itself on the new version (when built from the project, it keeps working while the new version is tested and built in the background). If something goes wrong, a message says why and the current version keeps working (details in %APPDATA%\Owlingstone\update.log).

## 1.8.2
- The Owlingstone server address is set: "Share my games" and "Send feedback" now work. If the server is offline, your games and feedback wait on your PC and leave with a later send.

## 1.8.1
- Right panel: the comps are now a tier list (S, A, B, C, D). The tier comes from the comp's average placement, using your hero's own result with it when your hero is one of its best, plus your rules (prefer / avoid). The comp you play is first in its tier. Click a tier title to fold it.

## 1.8.0
- Share my games (optional, asked once, "No" by default): at the end of a game, an anonymous summary (hero, placement, comp, lobby tribes, final board and each fight's boards: cards and stats only) is sent to the Owlingstone server. Never your BattleTag, other players' names, your logs or files. Games are sent together in one small request, at the end of a game or when the app starts; offline, they wait (30 days at most).
- Send feedback: right-click the tray icon > "Send feedback…" to send an idea, a bug or anything else (1000 characters). It leaves with the next send, even if you don't share your games.
- Tray menu: "Share my games" on/off, "Show what is sent" (the exact last summary), "Delete my shared games" (erases everything you sent from the server).
- Every day the app downloads the stats built from all shared games (from GitHub, not the server), ready for the advice to use once enough games are in.
- Fair stats: AFK games (3 turns in a row with no action at all; one or two quiet turns, hero-power-only turns and heroes that wait on purpose are fine) and concedes while still healthy are left out of the shared stats. Early concedes are still analysed in totals (lost streak, big hit, behind on tier, weak board, slow leveling, passive start...) to find the misplays the advice should warn about.
- The sharing switch and feedback stay greyed out until the server is online.

## 1.7.1
- Card data download fixed for new installs: HearthstoneJSON refused the app's download (403), so a fresh install had no cards. The app now names itself when downloading and only replaces cards.json once the download is complete.

## 1.7.0
- New name: **Owlingstone** (Owlingstone.exe, desktop shortcut "Owlingstone"). Your history and settings are copied from %APPDATA%\BGTracker to %APPDATA%\Owlingstone on first start; the old folder is left as it was. The old BG Tracker build and shortcut are removed by the update.
- Owlingstone is ready to be shared on GitHub (github.com/MimizukuDawn/owlingstone): README, full PolyForm Noncommercial license (free, never sold), privacy and security pages.
- Updates for everyone: a copy downloaded from GitHub checks for new versions in the background (every 6 hours), downloads the new one, checks its SHA-256 checksum and shows "Update to vX". Clicking it swaps the app folder and restarts; if anything fails, the old version stays. Your own copy built in the project keeps updating as before.
- Every push to GitHub runs the tests, a privacy scan (no BattleTag, e-mail, user path, private link or secret can be published) and security checks. Each version tag builds the Windows zip on GitHub with a checksum and a build attestation.
- The build now includes every data file (economy, hero curves, hero comps, trinkets), and the libraries are pinned to exact versions.

## 1.6.6
- Trinkets: when the Trinket offer appears (turn 6 Lesser, turn 9 Greater), the Comps tab shows the 4 Trinkets ranked by expected place, the best one in gold with ▶, and why under each one: its average place this patch (bgbuddy.gg, 264 Trinkets), whether its tribe is on your board or in your comp, whether the minion it gives (Portraits / Stickers) is a core card of your comp, synergy with your board and hero power, "pivot" discount (-2 gold), and "costs X, you have Y" when you can't afford it.
- Next Trinket offer (turns 5 and 8): the offer is made from your board and hand when the next turn starts. Tribe Trinkets need 2 of the tribe for the Lesser offer and 3 for the Greater one. The panel lists the tribes you already unlock with their best Trinkets, and which tribe 1 more minion would unlock ("+1 Undead unlocks: Eternal Portrait 3.19…"), so you can steer the offer before ending the turn.
- data/trinkets.yaml is made by tools/make_trinkets.py from tools/bgbuddy_trinkets.txt (copy the bgbuddy.gg table there after a patch).

## 1.6.5
- Hero curves: the Turn plan now knows heroes that play a different curve. Arch-Villain Rafaam uses his hero power every turn from turn 2 and goes Tier 2 on turn 4 ("Rafaam curve"); Forest Warden Omu levels a lot; Millhouse and Sindragosa buy minions for 2 gold; and so on for 38 heroes. Level up advice says "on / behind / ahead of <hero>'s curve".
- Gold for your hero power: when your hero power is worth using this turn, the plan keeps that gold aside ("Use I'll Take That! (1 gold, kept aside above)") and buys and levels with the rest.
- Best comps for your hero: at hero pick, each hero shows the comps it does best in (current patch, bgbuddy.gg) and its curve note. In game, the Turn plan shows your hero's best comps in this lobby, and the "Comps in this lobby" order uses your hero's results with each comp when known.
- Synergy now: a shop card that works with what is already on your board (same tribe as your "your Beasts" cards, Tavern spell makers with spell payoffs, Magnetic with Mechs, Deathrattle with Deathrattle payoffs...) gets points in Buy, even if it is not part of a comp ("synergy now: Mindslasher").
- data/hero_curves.yaml (curve, hero power use, notes) and data/hero_comps.yaml (best heroes per comp) can be edited freely; they are read again when they change.

## 1.6.4
- Right panel: click any section title (Turn plan, My hero power, Playing, Buy, Comps, Win chance, Opponent, Lobby) to fold or unfold it. ▾ = open, ▸ = folded. Folded sections stay folded in your next games.

## 1.6.3
- The combat bar (top of the game and Lobby tab) now always writes all three chances under it: Win %, Tie % and Loss %, like HDT. Before, a small part of the bar had no room for its number.
- The turn plan's next-fight line also shows the tie chance.

## 1.6.2
- Right panel fixed: it no longer grows past the screen or spills onto your other monitor. When the turn plan, buys and comps don't fit, the tab scrolls instead, and the panel always stays stuck to the right edge of the game.

## 1.6.1
- Economy and card advantage now count in the Turn plan. Cards that give gold, free rolls or Fodder (Accord-o-Tron, Malchezaar, Snare Trapper, Twisted Wrathguard...) and cards that give you more cards (Gearfin, Cataclysmic Harbinger, Bigwig Bandit, Rally / Deathrattle generators, Discovers...) get points in Buy, with the reason shown ("economy: +1 gold each turn", "cards: two 1-cost spells each turn"). They are worth more early in the game, when you have more turns to profit.
- Sell: keeps the cards that keep paying while on your board, and sells first the ones that pay when sold (Patient Scout, Tortollan Blue Shell...) or whose economy Battlecry is already used (Southsea Busker...).
- Level up: what you lose by leveling now counts the comp, stats and economy value of the buys you skip.
- The list is in data/economy.yaml (61 cards, value / when it pays / note): edit it freely, it is read again when it changes.

## 1.6.0
- New "Turn plan" at the top of the Comps tab while you shop: whether to level up, what to buy, what to sell, whether to roll or freeze, and how dangerous the next fight is for your health.
- It simulates your board (with the minions in your hand played) against the board your next opponent should have now, with each shop minion added, with all your gold spent, and with a level up. Damage is capped like in the game (cap shown), and you see the expected health loss and the chance the fight kills you.
- Your next opponent's board is no longer their old board as it was: boards grow about ×1.6 per turn, so their last board is "grown" to this turn (learned from your 13 games of this patch). Never fought them: a typical board for the turn. On your past re-matches this halved the error (the old estimate said you'd win 100% when you really won 32%). The "Next fight" bar and the top banner use it too.
- Buy suggestions now add what each card changes in the next fight (+% win, damage avoided) and the stats it adds for later fights to the comp / triple points. Cards your gold can't cover this turn are greyed.
- Level up: follows the usual curve (Tier 2 turn 2, 3 on turn 5, 4 on turn 7, 5 on turn 9, 6 around turn 12), but waits when it costs too much health or skips buys you need, and never when it can get you killed.
- Roll: tells you when to roll with your leftover gold (unspent gold is lost) and your chance per roll to see a card you want (missing core cards of your comp, third copy of a pair). Freeze when a key card is left in the shop.
- Sell: when your board is full, the weakest minion that isn't part of your comp or a pair.

## 1.5.3
- Combat win chance now also shows at the top of the game, above your opponent, like HDT (win / tie / loss bar, plus lethal warnings). Clicks go through it. Turn it off in the tray menu: "Win chance at the top".

## 1.5.2
- Both side panels are slimmer and stuck to the very edges of the game: the left one stays off the hero portraits, the right one off the board.
- Tray menu: "Side panels narrower" / "Side panels wider" to adjust them yourself (remembered).

## 1.5.1
- The left panel sits at the very left edge of the game and gets narrower when needed, so it no longer covers the hero portraits of the leaderboard.

## 1.5.0
- Combat win chance, like HDT's BobsBuddy: when a fight starts, the Lobby tab shows a bar with your chance to win / tie / lose, the average damage you deal or take, and a warning when the fight can kill you (or kill them). It replays the fight thousands of times with both real boards.
- Before the fight (while you shop), the same bar estimates your current board against your next opponent's last known board.
- "This game": your average win chance over all your fights, and how many you won compared with what was expected (good or bad luck).
- Checked on 109 of your past fights: when it said 90% or more you won every time, under 10% you never won, and the expected wins (37.7) were close to the real ones (36).
- Cards whose effect in combat isn't simulated yet are listed under the bar ("Not simulated").

## 1.4.1
- Your current hero power is shown at the top of the Comps tab (with its art and full card on hover) and the advice refreshes as soon as it changes.
- The game log is read 4 times per second instead of 2.
- rules.yaml can use hero_power: "Name" in a condition, to prefer or avoid comps with a given hero power.

## 1.4.0
- Hover any card or hero tile (tier list or right panel) to see the full card image next to the panel, like HDT.
- Top of the right panel: current turn, your tavern tier, your Tavern spell bonus (+attack/+health) and your Blood Gem value (shown when Quilboars are in the lobby or it has grown).

## 1.3.0
- New look, like Hearthstone Deck Tracker: every card is a tile with its art, a tier gem on the left and attack / health badges on the right. Gold border = in the shop, green = yours, blue = in hand.
- Comps tab: each comp is a guide card with badges (average place in green / yellow / red, difficulty, your rules, cards you own) and a strip of its cards' art. Open it to see every core and add-on card as a tile.
- The hero pick, "Need" and "Buy" are shown as tiles too.
- Lobby tab: hero portraits with health, tier, triples and the last fight result as badges; the opponent's board as card tiles.

## 1.2.0
- The right panel now has a Comps tab (always there during the game): what you're playing, the cards you still need, what to buy, and the hero pick.
- Every comp of this lobby, your preferred ones first (rules.yaml), with average place. Click one to see its core and add-on cards (✓ = you have it, gold = in the shop), when to commit and how to play. The open comp gets ★ in the tier list.
- Comps hidden by your rules are listed at the bottom. The Lobby tab keeps the opponents.

## 1.1.0
- New Lobby panel on the right of the game: your next opponent with the board they had last time you fought, their tier, health and triples, and a guess of the comp they play.
- Lobby list of the 8 players (place, health, tier, triples, last fight against you). Click a player to see their last board.
- Your fights this game (won / lost / tie) and today's games (places and average).
- One-click updates: when a new version is ready in the project folder, the app offers "Update now" (tray icon and window).
- Your data (history, settings, card images, learned races) now lives in %APPDATA%\BGTracker, so updates and rebuilds never touch it.
- Editing comps.yaml, rules.yaml or heroes.yaml applies within a few seconds, without restarting.

## 1.0.0
- Tier list overlay, comps and rules, hero and shop advice, game history and My stats, card art, tray icon.
