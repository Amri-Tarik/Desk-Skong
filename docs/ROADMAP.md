# Desk-Skong roadmap

_Working list, last updated 2026-11-19. Reorder freely and add notes under any item
(`> note: …`). `docs/STATUS.md` keeps the record of what's done and the known issues._

## 1. Next up: quick wins

- [x] **Hornet faces the knight.** _(Her talk clip looks left while pointing right: it is
      no longer mirrored, so her face turns to him; the needle points the other way.)_ Her clips face one way, so when the knight is on her
      other side she looks like she's ignoring him. Every visitor whose art faces one way
      should turn toward him while idling and talking.
- [x] **Re-sit quirk.** _(Fixed: a seat survives switching to idle / company / rest / nap;
      leaving it later is always graceful.)_ If the brain switches behaviour right after he sits, he gets up
      and sits straight back down. Fix: keep a running seated act when the next behaviour
      wants to sit too.
- [x] **Crystal dash on long runs.** _(Hurry crystal dashes along level runs longer than
      ~15 units ≈ 576 px at scale 0.6, the break-even with sprinting. If it feels too
      eager, add a margin in `superdash_break_even`. Follow-ups: wall-launched charge,
      runs to slightly higher ledges.)_

## 2. Reasons to move around

- [ ] **Enemies / training dummies** wandering in from the screen edges (enemy art from
      level files). Zote as a sparring partner. Fights are his top priority, so this pays
      off the most.
- [ ] **Geo showers through the subtle portal**, plus a tray/panel trigger for visits.
- [ ] **Dream Gate fallback.** When no route exists, he warps instead of giving up.
- [ ] **Hazards.** The spike/acid death acts exist but nothing triggers them. Needs a
      design: what counts as spikes or acid on a desktop?
- [ ] **Regional Geo rocks.** The game has City, Green, Fungal, Deep and Grave variants
      (found in `sharedassets86/27/168/188/245`). Pick one per app for variety.

## 3. Ideas and flavour

- [ ] **Reacting to on-screen text** (opt-in, local only). Windows UI Automation's
      "selection changed" event gives the highlighted text and where it is (Word, Notepad,
      Edge/Chrome; Electron apps may need accessibility switched on). Start with "curious
      about highlighted text", keyword reactions later.
      _(Step 1 done: tray/panel toggle "Curious about highlighted text", off by default. He
      walks to a spot near a fresh selection and looks at it, once; rectangles only, the
      text is never read. Not yet seen live.)_
- [ ] **Unn slug form** for very calm moods (28 unused `Slug *` clips).
- [ ] **Ability progression**: earn dash, wings, claw and crystal heart through the
      game's "collect" cutscenes. *Undecided.*
- [ ] **The game's particles recreated** (crystal shards, run/land dust).

## 4. Technical, low priority

- [ ] Nav graph builds are slower since climbing (~2.3 ms median, background thread).
      Next lever: share climb results per wall.
- [ ] Load animation frames only as needed, to cut memory at large scales.
- [ ] With "Visible in screen shares" (or `--capturable`) on, a rock or coin on a line
      that disappears can float until it's turned off.
- [ ] Rotated monitors get no content terrain.

## Needs a look on screen

These have only been checked in tests. Note anything that looks off:

- [ ] Step-behind crossings (TurnToBG / TurnFromBG at drawn lines)
- [ ] Pester tricks and their persistence (45–90 s)
- [ ] Sly's lesson: demo arcs vs. nearby windows; his nail vanishing at the bow
- [ ] Thought cloud behind the Geo / "?" emotes
- [ ] Seated map: open, read, close
- [ ] Geo Rock art and gleam; shiny glint and pick-up
- [ ] Cursor hit markers and the defeat burst
- [ ] Lantern in dark places (dark-theme detection thresholds)
- [ ] Fatal falls → Shade; fight mode; screen-share toggle

## Recently done (day 2, #51–#80)

Sit loop; walls and stuck routes; step behind drawn lines; pester; fused poses; thought
clouds; Geo from minimized windows; Nailsage Sly; strolls climb; deposits reachable,
mined and relocated (no regrowth); the game's Geo Rock; settled-ledge rule (15 s); shinies;
cursor play below Geo and visitors; hit markers; map and lantern back; fatal falls;
Ctrl+Alt+M menu; screen-share toggle; fight mode.
