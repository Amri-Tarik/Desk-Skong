# Desk-Skong roadmap

_Working list, last updated 2026-11-19 (night). Reorder freely and add notes under any item
(`> note: …`). `docs/STATUS.md` keeps the record of what's done and the known issues._

## 1. Next up

- [ ] **Training dummy** (`Training Dummy Real Anim`, sharedassets321: idle + light/heavy
      hit L/R). Something to practise on when bored, and a natural prop for Sly's lessons.
- [ ] **Pack the remaining art** (visitors, enemies, Geo rocks, bubbles) into the
      `FrameStore` like the knight and effects: most of the remaining ~180 MB private memory.
- [ ] **Small switches**: enemies on/off, a "Dream Gate now" debug button, a margin on how
      eager the crystal dash is (`superdash_break_even`).
- ~~Ability progression~~ — dropped: the point is seeing him use as many cool tools as
  possible, so he always has the full kit.

## 2. Reasons to move around

- [x] **Enemies**: Crawlid, Vengefly, Wandering Husk wander / fly / drop in every 7–25 min
      of presence (or when he's bored); he fights them above Geo and the cursor; they bump
      back (non-lethal by default), die and burst Geo. "Send an enemy" in the tray / panel.
      Zote may challenge him to a spar after a talk. Tuning: `data/enemies.json`, Zote's
      `spar`.
- [x] **Gauntlet**: an arena event: "!" and his `challenge`, 3 waves of 2–4 enemies (each
      after the last is cleared and a breather), a 40–60 Geo burst and his `victory`. Ends
      gracefully on hard typing, fullscreen apps, visitors, or when he's badly hurt. Rare on
      its own (after several single kills), or "Start a gauntlet" in the tray / panel.
      Tuning: the `gauntlet` section of `data/enemies.json`.
- [x] **Geo showers through the subtle portal**, settled ledges only; "Geo shower" in the
      tray / panel.
- [x] **Dream Gate fallback**: no route to a prize (visitors, the Shade, Geo, deposits,
      shinies) → he plants a gate and warps; settled ledges only; 90 s cooldown.
- [x] **Regional Geo rocks**: 10 regions, one per app (stable hash of the process name).
- [x] **Crystal dash on long runs** and **launched from walls** (Hurry only).
- ~~Hazards~~ — dropped (no desktop-native idea for spikes / acid yet).

## 3. Ideas and flavour

- [x] **Curious about highlighted text** (opt-in, off by default; rectangles only, the
      text is never read). Next step, keyword reactions, would need the text: a privacy
      decision.
- [x] **Unn slug form** for serene moments.
- [x] **The game's particles recreated** (dust bits, crystal shards, Geo sparkle).

## 4. Technical, low priority

- [x] Nav graph builds faster: collision queries test geometry before the window-overlap
      rule (soak median 2.3 → 2.0 ms, stress desktops −55 %, identical graphs).
- [ ] Maybe: share climb simulations per source/wall across targets. Measured ceiling ≤ 8 %
      of ticks on the soak desktops (walls serve ~1.4 targets), ≤ 18 % on stress desktops;
      needs a replay cheaper than cloning the character each tick to pay off.
- [x] Frames stay packed until drawn (knight + effects 75 → 22 MiB private at x0.6,
      202 → 41 at x1.0; live pet 228 → 183 MB private).
- [ ] With "Visible in screen shares" (or `--capturable`) on, a rock or coin on a line
      that disappears can float until it's turned off.
- [ ] Rotated monitors get no content terrain.

## Needs a look on screen

Checked in tests only (unless noted). Note anything that looks off:

- [ ] Step-behind crossings (TurnToBG / TurnFromBG at drawn lines)
- [ ] Pester tricks and their persistence (45–90 s)
- [ ] Sly's lesson: demo arcs vs. nearby windows; his nail vanishing at the bow
- [ ] Thought cloud behind the Geo / "?" emotes; seated map open / read / close
- [ ] Geo Rock art and gleam (all regions, sizes); shiny glint and pick-up
- [ ] Cursor hit markers and the defeat burst; fight mode; screen-share toggle
- [ ] Lantern in dark places (dark-theme detection thresholds)
- [ ] Fatal falls → Shade
- [ ] Combat kit: pogo chains (enemies, Zote, the cursor in battles and pester), dive from a window
      top, shadow-dash dodge through a Husk lunge, crystal-dash opener, wraiths at a Vengefly
- [ ] Enemies: feet line and facing, arrivals, hit flash, death and corpse fade, how often
      they bump him; Zote's spar. _(One Vengefly fight verified in the live logs.)_
- [ ] Gauntlet: the announcement, wave pacing and breathers, drop-in dust puffs, the reward
      burst and his `victory`; how often he finishes one
- [ ] Dream Gate: gate size, glow, warp timing
- [ ] Crystal dash: charge → launch on long runs and off walls; particles (dust, shards)
- [ ] Unn slug crawl speed and turns; highlighted-text curiosity with real apps

## Recently done

- Day 2, #51–#80: sit loop; walls and stuck routes; step behind drawn lines; pester; fused
  poses; thought clouds; Geo from minimized windows; Nailsage Sly; strolls climb; deposits
  reachable, mined and relocated; the game's Geo Rock; settled-ledge rule; shinies; cursor
  play below Geo and visitors; hit markers; map and lantern; fatal falls; Ctrl+Alt+M menu;
  screen-share toggle; fight mode.
- Day 2 night, #81–#95: roadmap; Hornet's facing; re-sit quirk; crystal dash on runs, from
  walls, and its charge no longer cancelled; portal Geo showers; regional rocks; particles;
  Unn slug; highlighted text; Dream Gate; enemies + Zote spar + "Send an enemy"; packed
  frames; faster nav builds.
