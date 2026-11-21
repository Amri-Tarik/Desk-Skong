# Desk-Skong

**The little knight from Hollow Knight, living on your Windows desktop.**

<!-- TODO(screenshot): a hero GIF of the knight running along window tops (~800 px wide). -->
<!-- ![The knight exploring the desktop](docs/media/hero.gif) -->

Desk-Skong is a small desktop companion. The knight walks along the tops of your windows
and your taskbar as if they were the caverns of Hallownest: he runs, jumps, dashes, sits on
benches, keeps you company while you work and, now and then, finds something to do.

- **He explores.** Window title bars, the taskbar, even lines drawn on screen (a table
  border, a stroke in Paint) are ledges to him. He moves the way he does in the game.
- **He finds things.** Benches, Geo rocks to mine, shiny trinkets, and grubs trapped in
  jars inside your apps, waiting to be set free.
- **He has visitors.** Cornifer, Quirrel, Myla, Zote, Elderbug, Hornet and Sly drop by to
  chat, draw maps, sing, spar, race or teach him something.
- **He fights.** Crawlids and friends wander in now and then. If he falls, his Shade keeps
  his Geo until he wins it back. Annoy him enough and he'll duel your mouse cursor.
- **He has hobbies** when it's quiet: a training dummy, chasing lumaflies, taking in the
  view, stargazing at night.
- **He has secrets:** hidden achievements, and something waiting once enough are found.
- **He stays out of your way.** You can pick him up and toss him aside. He never moves your
  mouse or your windows, and when a fullscreen game starts he leaves that screen.

<!-- TODO(screenshot): 2–3 stills side by side: a visitor conversation, a fight, the control panel. -->
<!-- ![A fight](docs/media/fight.gif) ![A visitor](docs/media/visitor.gif) ![The panel](docs/media/panel.png) -->
<!-- Placeholders for hero.gif, fight.gif and visitor.gif render locally into git-ignored
     docs/media/ (see docs/DEVELOPING.md, "README GIFs"). They are game art, so they
     can't be committed. -->

---

## What you need

- **Windows 10 or 11** (64-bit).
- **Your own copy of Hollow Knight**, installed on the same PC (Steam, GOG, Epic or Xbox).
  Desk-Skong doesn't include any of the game's pictures: it borrows them from *your*
  installed game the first time it runs (see [Hollow Knight and copyright](#hollow-knight-and-copyright)).

## Installing

1. Download `Desk-Skong-<version>-windows-x64.zip` from the [Releases](../../releases) page.
2. Unzip it anywhere you like, for example `Documents\Desk-Skong`. Nothing gets installed
   and you don't need administrator rights.
3. Open the folder and double-click **`skong.exe`**.

**"Windows protected your PC"?** Desk-Skong is a free hobby project and isn't signed with a
paid certificate, so Windows SmartScreen may warn you the first time. Click **More info →
Run anyway**. (If you'd rather not, you can build it yourself from this source code: see
[For developers](#for-developers).)

Keep the files in the folder together (`skong.exe`, `skong-panel.exe` and the `data`
folder). To update, unzip the new version over the old one: your settings and progress are
kept separately and carry over.

## The first run

The first time, a small window appears before the knight does:

1. **It looks for Hollow Knight** in your Steam libraries and the usual GOG, Epic and Xbox
   folders. If it can't find it, click **Locate hollow_knight.exe…** and pick the game's
   exe (on Steam: right-click Hollow Knight › *Manage* › *Browse local files*).
2. **It reads the knight's pictures from your game** (a few seconds). Your game is only
   read, never changed.
3. **The knight appears.** The pictures are kept in `%APPDATA%\Desk-Skong\skins\Vanilla`
   on your PC, so this doesn't happen again. After a game update, or if something looks
   off, use **Re-extract game art…** in the tray menu or the Desk-Skong window.

If you close that window before it's done, Desk-Skong simply doesn't start; run it again
any time.

## Using it

- **The Desk-Skong window** opens with him: your journal (achievements, Geo, time spent
  together) and the settings (size, how mischievous he is, what he's allowed to do).
- **The tray icon** (next to the clock) has the same switches. Click it to bring the window
  back.
- **Closing the window quits Desk-Skong.** To keep him around without the window, click
  *Hide to tray*. You can also quit from the tray icon or with `Ctrl+Alt+Q`.
- **Sandbox:** want to call a visitor, send an enemy or try something out right now? Turn on
  the sandbox in the window's *Sandbox* tab. Achievements pause until you restart.
- **Handy keys** (Ctrl+Alt + …): `W` wake him · `G` walk / `H` hurry to the mouse · `P`
  pause · `M` the tray menu · `Q` quit.
- **Skins:** community "Custom Knight" skins (a folder with a `Knight.png`) work too; pick
  one from the tray.

## Is it safe? Your privacy

An app that watches your screen deserves some questions. Short answers:

- **It never uses the internet.** Desk-Skong doesn't connect to anything: it sends nothing,
  tracks nothing, has no account and never "phones home". It works the same with your
  internet unplugged.
- **It doesn't record your screen.** The knight has to "see" your windows to know where he
  can walk, so Desk-Skong glances at the screen to find edges and lines. Each glance is
  forgotten immediately: nothing is saved, nothing is sent, and no text is read. Don't
  want even that? Switch off *Stands on lines inside windows* and he only walks on title
  bars and the bottom of the screen.
- **It doesn't read what you type.** It only notices *that* you're typing, so the knight
  doesn't bother you while you work. It never knows which keys you press.
- **It doesn't read your text.** When you highlight text, the knight comes to look, but
  Desk-Skong only knows *where* the highlight is on screen, never what it says (and never
  in password boxes). You can switch this off too (*Curious about highlighted text*).
- **What it does know:** where your windows are, their titles and which apps they belong
  to, and where your mouse pointer is. That's how the knight finds his way around (and a
  few visitors' jokes mention an app or a window title). It all stays on your PC.
- **What it keeps:** your settings, the knight's progress and the pictures borrowed from
  your Hollow Knight, in one folder on your PC (`%APPDATA%\Desk-Skong`). Log files only
  if you switch logging on. Nothing leaves your PC unless you send a file yourself.
- **It doesn't install itself.** No installer, no administrator rights, nothing added to
  your startup, no Windows settings changed. Delete the folder and it's gone.
- **He stays out of screenshots and video calls** unless you switch on *Visible in screen
  shares*.

## Reporting a bug

Bugs and ideas are welcome in [Issues](../../issues). A log helps a lot:

1. Turn on **Write a log file (for bug reports)** (tray icon or the Desk-Skong window).
2. Make the bug happen again.
3. Tray → **Open log folder** (`%APPDATA%\Desk-Skong\logs`) and attach the newest
   `skong-<date>.log`, plus any `crash-….log` or `trace-….log` (crash reports and traces
   are written there even with logging off).

Have a quick look before you share them: logs mention window titles and program names.

## Uninstalling

Delete the folder you unzipped. To also forget his progress and the extracted pictures,
delete `%APPDATA%\Desk-Skong`. That's everything.

---

## Hollow Knight and copyright

**Desk-Skong is an unofficial fan project. It is not affiliated with, endorsed by or
connected to Team Cherry. Hollow Knight, its characters, names, art, music and story are
the property of Team Cherry.**

- **No game files are distributed.** Neither the download nor this source code contains any
  art, animation, sound, music or text from Hollow Knight. The automated release build
  checks every package and refuses to ship any image, audio or game file.
- **The art comes from your own copy.** On first run, Desk-Skong reads the knight's pictures
  from the Hollow Knight installed on *your* PC (which you need to own) and keeps them there,
  for your personal use. They are never uploaded or shared. Please don't share the extracted
  pictures either.
- **The writing is original.** Every line the visitors say, the knight's thoughts and the
  achievement texts were written for Desk-Skong; none of it is copied from the game.
- **What the project does include:** its own code, and small descriptions of *where* things
  are in the game's files (sprite positions and animation timings, no pictures), so the art
  can be found on your PC.

If you enjoy the knight, play (or replay) [Hollow Knight](https://www.hollowknight.com/)
and support the people who made it.

## Licence and credits

The Desk-Skong code is released under the **MIT License** (see [`LICENSE`](LICENSE)). That
covers the code only: Hollow Knight's art, sound and text belong to Team Cherry and are not
part of this project.

- **Hollow Knight** and everything in it: © Team Cherry.
- **Fonts** (built into the programs): [Cinzel](https://github.com/NDISCOVER/Cinzel) and
  [Crimson Pro](https://github.com/Fonthausen/CrimsonPro), both under the SIL Open Font
  License 1.1 (see `FONTS-OFL.txt`).
- **Built with** Rust and open-source libraries including [egui](https://github.com/emilk/egui)
  (the window) and the [`windows`](https://github.com/microsoft/windows-rs) crate; their
  licences travel with their source in the usual Rust package registry.
- **The developer art exporter** (`HK_TK2D_Exporter`, not part of the download) uses
  [AssetsTools.NET](https://github.com/nesrak1/AssetsTools.NET) and its Unity type database
  (`classdata.tpk`, from the [AssetRipper Tpk](https://github.com/AssetRipper/Tpk) project),
  and [StbImageWriteSharp](https://github.com/StbSharp/StbImageWriteSharp).

---

## For developers

Building from source, exporting the art for development, architecture, tests, CI and
releasing: see **[docs/DEVELOPING.md](docs/DEVELOPING.md)**.

- Architecture & internals: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Feature status & backlog: [docs/STATUS.md](docs/STATUS.md), [docs/ROADMAP.md](docs/ROADMAP.md)
- Asset exporter: [HK_TK2D_Exporter/readme.md](HK_TK2D_Exporter/readme.md)
