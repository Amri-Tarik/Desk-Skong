# Desk-Skong roadmap

_Working list, last updated 2026-11-20. Reorder freely and add notes under any item
(`> note: …`). `docs/STATUS.md` keeps the record of what's done and the known issues._

## 1. Fixes and unwanted behaviour (first)

- [ ] **The Shade really comes first.** Its priority is right (Urgent), but timers hold it
      back:
      - after a respawn he waits 90 s (`shade_delay`) before going after it;
      - every failed attempt (unreachable, route failed, stuck, the 60 s fight limit)
        benches it for 3–6 min (`shade_retry`).

      Now that he respawns far away (often on the other monitor), "unreachable" is common.
      Plan:
      - only a few seconds of delay after a respawn;
      - never shelved for minutes: if there's no route, take the Dream Gate to it, or retry
        within seconds;
      - only dying ends an attempt.
- [ ] **Geo left behind after a kill.** Two suspects:
      1. **Worn out:** after a rough fight, Geo counts as a pastime unless a coin is right
         next to him, and recovering from hurt outranks it. He rests while the coins fade
         (they last 4 min).
      2. **Coins still bouncing:** when the fight ends there's no resting Geo yet, so
         something else starts. If that's a Geo deposit, it doesn't give way when the coins
         land.

      Fix: only sleep may push Geo down, and rest waits until the coins are collected.
      Reproduce both in a simulation first.
- [ ] **Stutter with several monsters** (walks, stops, walks, very fast, so he crawls
      along). The target is sticky, so it's not target swapping. Suspects:
      - another monster in the way bumps him each step;
      - combat-kit moves (jump at a foe, dodge) cutting the walk short as their cooldowns
        expire.

      Reproduce with 2–3 foes in a simulation. Fix: a foe in the way becomes the target,
      and each decision is held for a minimum time.
- [ ] **Staring at the user** uses the walk-towards animation: hold a single frame instead.
- [ ] **Picking him up is always enabled.** No toggle. He's a gremlin you drag out of the
      way, and he does **not** dodge your typing on his own.
- [ ] **Highlighted-text curiosity on by default**, with a toggle to turn it off.

## 2. Expression and variety

- [ ] **Expression bubbles** (small, in an HK style, never noisy):
      - "…" when bored;
      - an anger mark when there are enemies or a cursor to fight;
      - "!" on spotting the Shade, Geo or a flame;
      - "?" when he can't reach something;
      - "z" when drowsy;
      - a heart after a nice visitor chat.
- [ ] **Less time in companion mode.** More things to do on his own:
      - the training dummy (`Training Dummy Real Anim`, sharedassets321);
      - chasing a lumafly across windows;
      - sitting on a window edge taking in the view;
      - swapping charms on a bench;
      - stargazing at night;
      - the lantern raised on dark-themed windows (exists; tune how often it happens).

## 3. Visitor scenes

- [ ] **Cornifer:** they sit down together and both open their maps for a while, reviewing
      them (he hums).
- [ ] **Grimm, friendly:** appears with a flourish, bows, taunts, teleports a couple of
      times and leaves. Sometimes he leaves the scarlet flame behind as a hint at the
      secret.
- [ ] **Hornet: race and duel, both** (50/50 at first; drop one if it doesn't hold up).
      - Race: she throws her needle and silk to mark the finish, and they race across the
        windows.
      - Duel: a short spar; she wins and leaves.
- [ ] **Quirrel** sits on a ledge beside the knight, admiring the view.
- [ ] **Myla** sings while mining next to a Geo deposit.
- [ ] **New achievements** for each scene.

## 4. Achievements: everyone can reach 100 %

- [ ] **Remove anything that needs specific hardware or software:**
      - multi-monitor ones: "Border Crossing" (crystal dash across monitors) and "Air Mail"
        (thrown onto another monitor);
      - review "Not My Kind of Game" (needs a detected fullscreen game);
      - review anything else tied to hardware or a particular app.
- [ ] Achievements stay locked in sandbox sessions (see 5).

## 5. Public release (GitHub), last: ship a more complete pet first

- [ ] **Legit install required, art extracted locally.** No game art or text ever ships.
      - On first run, look for Hollow Knight at the default Steam location (all Steam
        libraries); otherwise ask the user to pick the game's exe or folder.
      - Extract the sprites into the user's data folder.
      - The README and first-run screen say plainly that a legitimate Hollow Knight install is
        required.
- [ ] **Extractor in Rust.** The .NET exporter becomes a dev-only tool, only needed for
      regenerating metadata. Players get a single exe. The tk2d metadata JSON already in
      `data/hk` stays; only the atlas reading and cropping move into Rust.
- [ ] **Panel = the app.**
      - Closing the panel quits the pet; there's no separate exit button.
      - A clear "Hide to tray" button, with a one-line hint on how to bring it back and how
        to quit.
- [ ] **Front page:**
      - achievements (count, hints, unlocked list);
      - the "Visible in screen shares" (capturable) toggle;
      - Hide to tray.
- [ ] **Sandbox, behind a consent gate.** All "make him do X" buttons and debug tools live
      behind "Enable sandbox: achievements are disabled for the rest of this session".
      This covers enemies, visitors, gauntlet, Grimm, flame, showers and the Dream Gate.
- [ ] **Sandbox placement:** click on the screen to choose where things spawn.
- [ ] **Release basics:**
      - a portable zip (an installer later, if needed) and GitHub releases with notes;
      - a "fan project, not affiliated with Team Cherry" notice;
      - an opt-in crash/log file the user can attach to an issue;
      - settings and achievements that survive updates.
- [ ] Make the repo page public when ready.

## 6. Parked

- Geo spending (shop, maps): Geo stays a score until a good idea shows up.
- ~~Ability progression~~: he always has the full kit.
- ~~Hazards~~: no desktop-native idea yet.
- ~~Avoiding the text cursor~~: dragging him away is the intended answer.

## 7. Technical, low priority

- [ ] **Pack the remaining art** (visitors, enemies, Geo rocks, bubbles, Grimm) into the
      `FrameStore` like the knight and effects.
- [ ] **Small switches:**
      - enemies on/off;
      - how eager the crystal dash is (`superdash_break_even`).
- [ ] **Climb simulations:** maybe share them per source and wall across targets (≤ 8 % of
      ticks on the soak desktops).
- [ ] **Screen-share mode:** with "Visible in screen shares" on, a rock or coin on a line that
      disappears can float until it's turned off.
- [ ] **Rotated monitors** get no content terrain.

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
- [ ] Combat kit:
      - pogo chains (enemies, Zote, the cursor in battles and pester);
      - dive from a window top;
      - shadow-dash dodge through a Husk lunge;
      - crystal-dash opener;
      - wraiths at a Vengefly.
- [ ] Gauntlet: wave pacing, drop-in dust puffs, the reward burst; how often he finishes one
- [ ] Dream Gate: gate size, glow, warp timing
- [ ] The Troupe Master:
      - his size, feet line and facing;
      - teleport flashes and the lantern's flicker;
      - firebat / fireball / pillar scale and orientation;
      - the pillar glow and spikes;
      - how often he wins on a real desktop.

## Recently done

- Day 2, #51–#80: sit loop; walls and stuck routes; step behind drawn lines; pester; fused
  poses; thought clouds; Geo from minimized windows; Nailsage Sly; strolls climb; deposits
  reachable, mined and relocated; the game's Geo Rock; settled-ledge rule; shinies; cursor
  play below Geo and visitors; hit markers; map and lantern; fatal falls; Ctrl+Alt+M menu;
  screen-share toggle; fight mode.
- Day 2 night, #81–#112:
  - roadmap; Hornet's facing; re-sit quirk;
  - crystal dash on runs and from walls;
  - portal Geo showers; regional rocks; particles; Unn slug; highlighted text; Dream Gate;
  - enemies, Zote spar, packed frames, faster nav;
  - strict priorities, greedy Geo, Shade first;
  - window dreams as system jokes;
  - gauntlet; 32 hidden achievements; the combat kit (pogo, dive, dodge, situational
    spells); the secret Troupe Master;
  - borderless games detected as fullscreen.
