# Desk-Skong

**The Knight from Hollow Knight, living on your desktop.**

<!-- TODO(screenshot): a hero GIF of the knight running along window tops (~800 px wide). -->
<!-- ![The knight exploring the desktop](docs/media/hero.gif) -->

Desk-Skong is a small companion for Windows. The little knight explores your windows
and taskbar like they're Hallownest: he runs along title bars, wall-jumps, dashes, rests
on benches, keeps you company while you work and, now and then, has something to deal
with.

- **Explores your desktop.** Window tops, the taskbar and even lines on screen (table
  borders, a stroke in Paint) are platforms. He moves with the game's own moves: dashes,
  wall jumps, the crystal dash, the dive.
- **Fights.** Crawlids and friends turn up now and then; he takes them on with his nail
  and spells. Get on his nerves and he'll duel your cursor.
- **Collects Geo.** Geo deposits hide inside your apps; closed windows can shatter into
  coins. If he falls, his Shade waits for him to win his Geo back.
- **Meets visitors.** Cornifer, Quirrel, Myla, Zote, Elderbug, Hornet and Sly stop by to
  talk, map, sing, spar or teach him a thing or two.
- **Has secrets.** Hidden achievements, and something waiting once enough of them are
  found. We won't spoil it.
- **Stays out of your way.** Pick him up and toss him aside; he never moves your cursor
  or your windows, and when a fullscreen game starts he steps off that screen.

<!-- TODO(screenshot): 2–3 stills side by side: a visitor conversation, a fight, the control panel. -->
<!-- ![A visitor](docs/media/visitor.png) ![A fight](docs/media/fight.png) ![The panel](docs/media/panel.png) -->

---

## Requirements

- **Windows 10 or 11** (64-bit).
- **A legitimately owned copy of Hollow Knight** (Steam, GOG, …), installed on the same PC.
  Desk-Skong contains **no** art, sound or text from the game: the knight's sprites are
  read from *your* installation the first time you run it and kept on your PC.

## Install and run

1. Download `Desk-Skong-<version>-windows-x64.zip` from the
   [Releases](../../releases) page.
2. Unzip it anywhere you like (e.g. `Documents\Desk-Skong`). It's portable: nothing is
   installed, no admin rights needed.
3. Run **`skong.exe`**.

Keep `skong.exe`, `skong-panel.exe` and the `data` folder together. To update, unzip the
new version over the old one (or anywhere else): your settings, Geo, finds and achievements
are kept in `%APPDATA%\Desk-Skong` and carry over.

## First run: your Hollow Knight install

Desk-Skong needs your own copy of Hollow Knight. The first time you start it, a small
window reads the game's sprites from your installation, once, and keeps them on this PC
only:

1. **It looks for the game** in every Steam library (and the usual GOG, Epic and Xbox
   folders). If it can't find it, click **Locate hollow_knight.exe…** and pick the game's
   exe (Steam: right-click Hollow Knight › *Manage* › *Browse local files*).
2. **It extracts the art** (a few seconds), then the knight appears. Nothing in the game
   folder is changed: it's only read.
3. **The art is saved in** `%APPDATA%\Desk-Skong\skins\Vanilla`, next to your settings, so
   updating Desk-Skong doesn't need it again. After a game update (or if something looks
   wrong), use **Re-extract game art…** in the tray menu or the Desk-Skong window.

If the window is closed before it's done, Desk-Skong doesn't start; run it again any time.

> **Legal.** Desk-Skong requires a legitimately owned copy of Hollow Knight. Nothing from
> the game (art, sound or text) is distributed with Desk-Skong, and the extracted art is
> for your own use on your own PC: please don't share it. Desk-Skong is an unofficial fan
> project, not affiliated with or endorsed by Team Cherry; Hollow Knight and its art
> belong to Team Cherry.

## Using it

- **The Desk-Skong window** (the control panel) opens with the pet: achievements, your
  Geo, and the settings.
- **The tray icon** (notification area, by the clock) has the same switches: size,
  skins, mischief, pause, *Visible in screen shares*; click it (or *Open Desk-Skong*) to
  bring the window back.
- **Closing the panel window quits Desk-Skong.** To keep him around without the panel,
  use *Hide to tray*; bring it back from the tray icon. You can also quit from the tray
  icon (*Quit*) or with `Ctrl+Alt+Q`.
- **Sandbox:** to make him do things on request (call visitors, enemies, Grimm, Geo
  showers, drive him with the keyboard, choose where things appear), enable the sandbox
  in the window's *Sandbox* tab. Achievements are paused until you restart Desk-Skong.
- Handy keys (Ctrl+Alt + …): `W` wake him · `G` walk / `H` hurry to the cursor · `P`
  pause · `M` the tray menu · `Q` quit.
- **Skins:** community "Custom Knight" skins (a folder with `Knight.png`) work too; pick
  one from the tray.

## Privacy

Everything stays on your PC. Desk-Skong makes **no network connections**.

- **Screen content** is analysed live, in memory, to find ledges he can stand on (lines,
  borders, edges of things). Screenshots are never saved, never sent anywhere, and no
  text is read from them.
- **Highlighted text:** he's curious when you select text, but only the *rectangle* of
  the selection is used (from Windows' accessibility interface); the text itself is never
  read or stored. You can turn this off (*Curious about highlighted text*).
- **Typing and mouse:** only whether you're typing (a rate, not which keys) and where
  the cursor is, so he doesn't get in the way.
- **Saved on your PC** in `%APPDATA%\Desk-Skong`: settings, Geo and the Shade, achievements,
  and the things he found in your apps (remembered by program name, e.g. `chrome`).
- By default he hides from screenshots and screen shares; turn on *Visible in screen
  shares* if you want to show him off.

## Reporting a bug

Bugs and ideas are welcome in [Issues](../../issues). A log helps a lot:

1. Turn on **Write a log file (for bug reports)** (tray icon or control panel).
2. Make the bug happen again.
3. Tray → **Open log folder** (`%APPDATA%\Desk-Skong\logs`) and attach the newest
   `skong-<date>.log`, plus any `crash-….log` (crash reports are written there even with
   the log file off).

These files stay on your PC unless you attach them yourself. Have a quick look before
you share: they mention window and program names.

## Uninstall

Delete the folder you unzipped, and `%APPDATA%\Desk-Skong` if you want to forget his
progress too.

---

## Fan project

**Desk-Skong is an unofficial fan project, not affiliated with or endorsed by Team
Cherry. Hollow Knight and its characters, art and music are property of Team Cherry.**

If you enjoy the knight, play (or replay) [Hollow Knight](https://www.hollowknight.com/)
and support its creators.

## Licence

<!-- TODO(owner): choose a licence for the Desk-Skong code and add a LICENSE file at the
     repository root (the release zip picks it up as LICENSE.txt). The workspace
     Cargo.toml currently says `license = "MIT"`, but there's no LICENSE file: confirm or
     change that. Whatever is chosen covers the code only, never Team Cherry's game. -->

**TODO:** the licence for the Desk-Skong code hasn't been chosen yet. Until it is, all
rights are reserved. Hollow Knight's art, sound and text are not part of this project
and are never distributed with it.

---

## For developers

Building from source, exporting the art for development, architecture, tests, CI and
releasing: see **[docs/DEVELOPING.md](docs/DEVELOPING.md)**.

- Architecture & internals: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Feature status & backlog: [docs/STATUS.md](docs/STATUS.md), [docs/ROADMAP.md](docs/ROADMAP.md)
- Asset exporter: [HK_TK2D_Exporter/readme.md](HK_TK2D_Exporter/readme.md)
