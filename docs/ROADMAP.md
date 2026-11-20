# Desk-Skong roadmap

_Working list, last updated 2026-11-20. Reorder freely and add notes under any item
(`> note: …`). `docs/STATUS.md` keeps the record of what's done and the known issues._

## 1. Fixes and unwanted behaviour (first)

- [x] **The Shade really comes first.** Its priority is right (Urgent), but timers hold it
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

      > done: 4 s after a respawn; no route → the hunt plants its own Dream Gate (whatever
      > the gate's cooldown), else another look every 2–5 s; stuck / failed route / out of
      > reach → re-plan; no time limit; a 25 s breather after the Shade kills him.
- [x] **Geo left behind after a kill.** Two suspects:
      1. **Worn out:** after a rough fight, Geo counts as a pastime unless a coin is right
         next to him, and recovering from hurt outranks it. He rests while the coins fade
         (they last 4 min).
      2. **Coins still bouncing:** when the fight ends there's no resting Geo yet, so
         something else starts. If that's a Geo deposit, it doesn't give way when the coins
         land.

      Fix: only sleep may push Geo down, and rest waits until the coins are collected.
      Reproduce both in a simulation first.

      > done: both fixed, plus the live case (loot utility 0 with coins lying there): coins
      > were filtered by the shared "unreachable target" map (64 px cells), so a failed
      > seat / cursor spot / fight goal nearby hid fresh Geo for 20–60 s. Coins now have
      > their own skip list. Only sleep puts Geo down; a walk to a deposit gives way to
      > loose coins (a rock being mined is finished first).
- [x] **Stutter with several monsters** (walks, stops, walks, very fast, so he crawls
      along). The target is sticky, so it's not target swapping. Suspects:
      - another monster in the way bumps him each step;
      - combat-kit moves (jump at a foe, dodge) cutting the walk short as their cooldowns
        expire.

      Reproduce with 2–3 foes in a simulation. Fix: a foe in the way becomes the target,
      and each decision is held for a minimum time.

      > done (needs eyes on screen): the sim showed fight ↔ loot / recover ping-pong — a
      > bump took soreness past 0.6, the running fight lost its tier, Geo cut in, then the
      > fight took over again as he healed. Soreness now only blocks *starting* a fight.
      > An enemy right in front of him, between him and his target, becomes the target.
      > The exact in-app walk/stop rhythm wasn't reproduced (the sim has no path follower).
- [x] **Staring at the user** uses the walk-towards animation: hold a single frame instead.
- [x] **Picking him up is always enabled.** No toggle. He's a gremlin you drag out of the
      way, and he does **not** dodge your typing on his own.

      > done: the click-through toggle (tray, panel, Ctrl+Alt+C) was what blocked grabs; gone.
- [x] **Highlighted-text curiosity on by default**, with a toggle to turn it off.
- [x] **Talk to visitors from close enough.** A talk needs no contact: when he can't get
      right beside a visitor (a busy window), he talks from a reachable spot within range
      (6 units across, 5 up or down), same level first. Gives up only with no such spot.

## 2. Expression and variety

- [x] **Expression bubbles** (small, in an HK style, never noisy):
      - "…" when bored;
      - an anger mark when there are enemies or a cursor to fight;
      - "!" on spotting the Shade, Geo or a flame;
      - "?" when he can't reach something;
      - "z" when drowsy;
      - a heart after a nice visitor chat.

      > note: decided in `ai::expression` (brain transitions + need latches, per-bubble
      > cooldowns, a 6 s gap between soft ones) and sent as `Intent::emote`; drawn in
      > `emotes.rs`. Also a sweat drop when badly hurt and the sparkle for a big Geo haul
      > (40+ Geo within 12 s). Spotting Geo keeps the Geo-coin thought cloud rather than a
      > "!".
- [x] **Less time in companion mode.** _(Approved live 2026-11-20: "a lot more interesting".)_ More things to do on his own (`ai::pastime`, Ambient:
      the Shade, fights, visitors, Geo, shinies and deposits always come first). They come up
      after ~25 s of plain company (or when bored stiff), each with a cooldown. In a 30-min
      simulated desk session plain company went from ~91 % to ~67–72 %.
      - [x] the training dummy (`Training Dummy Real Anim`, exported into the vanilla skin;
            a drawn stand-in without it): stands on a settled ledge every 4–10 min for
            7–13 min; he practises combos, up slashes, pogos, great / cyclone / dash slash,
            fireballs; it swings per blow;
      - [x] chasing a lumafly across windows (a drawn glow): he follows it along the ledges
            under it, jumps at it, sometimes catches it and watches it float off;
      - [x] sitting on the end of a high window edge taking in the view;
      - [ ] swapping charms on a bench;
      - [x] stargazing at night (21:00–05:00, the highest open ledge, Look At King);
      - [ ] the lantern raised on dark-themed windows (exists; tune how often it happens).
            > note: now a setting, "Lantern on dark backgrounds" (tray + panel, on by
            > default, saved): off, the lantern never comes out; the map is unaffected.

## 3. Visitor scenes

- [x] **Cornifer:** they sit down together and both open their maps for a while, reviewing
      them (he hums). Standing with the map where there's no seat (the desktop floor).
      Calm scenes are `company` in `npcs.json` (`npc::company`, the brain's `Company`).
- [x] **Grimm, friendly:** appears with a flourish, bows, taunts, teleports a couple of
      times and leaves. Sometimes he leaves the scarlet flame behind as a hint at the
      secret (only once its achievements are unlocked).
      > done: `enemy::grimm::cameo`, rare (`cameo.schedule`), panel Debug "Grimm's friendly
      > visit"; achievement "A Passing Troupe".
- [x] **Hornet: race and duel, both** (50/50 at first; drop one if it doesn't hold up).
      - Race: she throws her needle and silk to mark the finish, and they race across the
        windows.
      - Duel: a short spar; she wins and leaves.
      > done: `npc::race` / `npc::duel`, 60 % of her talks end in one; panel Debug
      > "Hornet: race / duel". The knight wins ~1 race in 4 in the sim (`pace`).
      > Achievements "Faster Than Silk" (win a race), "Needle Practice" (see a duel through).
- [x] **Scarlet flame, clicked by the user:** the knight no longer strikes it; he lingers by
      it wondering ("?", a pastime), the user clicks it to summon Grimm right there, and it
      waits ~30 min (`lantern.secs`) so an arena can be built around it.
- [x] **Quirrel** sits on a ledge beside the knight, admiring the view (window edges in the
      upper half of a monitor only).
- [x] **Myla** sings while mining next to a Geo deposit (new visitor, art exported from the
      `Miner` library; she arrives beside a deposit when there is one, else visits plainly).
- [x] **New achievements** for each scene: "Comparing Notes", "Room for Two", "Work Song",
      "Faster Than Silk", "Needle Practice", "A Passing Troupe".

## 4. Achievements: everyone can reach 100 %

- [x] **Removed anything that needs specific hardware or software:** "Border Crossing" and
      "Air Mail" (two monitors), "Not My Kind of Game" (a detected fullscreen game). The
      rest only need the knight and ordinary windows. A test guards this.
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
      > note: the panel's **Test** tab (the sandbox's precursor, no gate yet) has it: 📍 next
      > to the dummy, flame, deposit, shiny, bench, each enemy, Geo shower, visitors and a
      > Dream Gate target; plus forced visitor scenes, each enemy kind, the view, the stars,
      > the slug, the emote cycle. Checklist: [TESTING.md](TESTING.md).
- [ ] **Release basics:**
      - a portable zip (an installer later, if needed) and GitHub releases with notes;
      - a "fan project, not affiliated with Team Cherry" notice;
      - an opt-in crash/log file the user can attach to an issue;
      - settings and achievements that survive updates.
      > note: built on `build_feat-release-basics`: player README (screenshot spots,
      > requirements, privacy, fan notice), `scripts/package.ps1` (zip with an art guard),
      > CI + a tag-triggered draft release, the opt-in log file + crash reports, atomic
      > saves with `.bak` for malformed files, settings that keep unknown fields. Open:
      > **choose a licence** (Cargo says MIT, no LICENSE file), screenshots/GIFs.
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

Checked in tests only (unless noted). Note anything that looks off. The full manual
checklist, with how to trigger each from the panel's Test tab, is [TESTING.md](TESTING.md):

- [ ] Step-behind crossings (TurnToBG / TurnFromBG at drawn lines)
- [ ] Pester tricks and their persistence (45–90 s)
- [ ] Sly's lesson: demo arcs vs. nearby windows; his nail vanishing at the bow
- [ ] Thought cloud behind the Geo / "?" emotes; seated map open / read / close
- [ ] Geo Rock art and gleam (all regions, sizes); shiny glint and pick-up
- [ ] Cursor hit markers and the defeat burst; fight mode; screen-share toggle
- [ ] Lantern in dark places (dark-theme detection thresholds)
- [ ] Carried map / lantern (`ai::carry`; the old `map_stroll` / `lantern_patrol` patrol acts
      are gone): following the cursor with Map Walk / Lantern Run, standing in Map Idle /
      Lantern Idle; Map Open → Map Idle and Map Away → Idle without a snap; Map Turn when he
      turns in place; the map folded before a fight / hurrying / sitting, dropped mid-jump;
      the lantern popping off over acts and jumps (game-faithful, no clips for it); how
      often the map comes out (every ~2 min, 15–40 s) and whether the lantern being out
      the whole time on a dark-themed desktop feels right
- [ ] Dummy / lumafly now beat plain company at once (utility 0.75–0.79 vs companion
      ≤ 0.62): does he run off to them too eagerly; does the summoned flame still win
- [ ] Fatal falls → Shade
- [ ] Combat kit:
      - pogo chains (enemies, Zote, the cursor in battles and pester);
      - dive from a window top;
      - shadow-dash dodge through a Husk lunge;
      - crystal-dash opener;
      - wraiths at a Vengefly.
- [ ] Reactions to handling (`ai::handling`):
      - a click: he turns to the cursor for 1.5 s (only when standing still in a pastime —
        does the turn read as "looking at you"?); a 2nd click within 25 s: anger mark;
      - 5 clicks within 8 s: the provoked cursor battle (challenge, then the fight; reason
        "battle: provoked, …"); 7 while typing; not over Geo / visitors / the Shade / fights;
        is 5 the right number, and is 15 s the right time for a provocation to wait;
      - a click while napping: wakes with an anger mark, no running off any more;
      - shaking him while held (4 swings of 40 px within 2 s): "?" and the Stun clip once he
        lands — is the threshold too easy / too hard with a real mouse, and does Stun read
        as dizzy;
      - 3 throws within a minute: anger mark, keeps the polite distance from the cursor for
        90 s, no battles / pestering meanwhile, more mischief after;
      - a gentle carry (held ≥ 0.3 s, let go < 250 px/s): nothing, or now and then a heart.
- [ ] Gauntlet: wave pacing, drop-in dust puffs, the reward burst; how often he finishes one
- [ ] Dream Gate: gate size, glow, warp timing
- [ ] Pastimes (tray → Discoveries (debug) → "Put up a training dummy" / "Let a lumafly
      drift by"):
      - the dummy's size beside the knight (`DUMMY_ART_SCALE` 0.34), its foot on the ledge,
        and whether the hit clips swing *away* from the blow (L / R mapping is a guess);
      - the drills against it: slash / pogo reach, the dash slash not carrying him off the
        ledge, hit markers lining up with the figure;
      - the lumafly glow (size, colour, bob), how catchable it feels, the catch flare;
      - the view: which end of the window edge he picks and which way he faces;
      - stargazing: Look At King held for 16–28 s on a high ledge;
      - how often they happen versus keeping company.
- [ ] The Troupe Master:
      - his size, feet line and facing;
      - teleport flashes and the lantern's flicker;
      - firebat / fireball / pillar scale and orientation;
      - the pillar glow and spikes;
      - how often he wins on a real desktop.
- [ ] Expression bubbles (`--emote-test` cycles them all):
      - the drawn marks at every Size: the anger mark's shape (four arcs), the heart, the
        sweat drop, the "…" cloud; their colours on light and dark windows;
      - the short fade in / out (four steps) and the sweat drop sliding down;
      - how often they show over a real day (the brain tests allow at most ~1 a minute);
      - the "z" when he settles for a nap vs. the "zzz" once asleep.
- [ ] Calm scenes (`--visit cornifer|quirrel|myla`; a scene follows the talk 50–70 % of the time):
      - Cornifer: the knight seated beside him with the map open (the gap between them, his
        hum bubbles over the drawing idle); the standing map on the floor;
      - Quirrel: both on a high window edge (Bench Look facing, Sit Lean pose), Bench Look End;
      - Myla: her size and feet line (new art), facing the rock while mining, standing beside
        a deposit without overlapping it or the knight's mining spot; the song bubbles' pace;
      - the abandon line when typing / an enemy / the Shade cuts a scene short.
- [ ] The scarlet flame, clicked: the flame's window takes the click (only it; the rest stays
      click-through), the knight's "?" every 7–13 s while he lingers by it, his pacing, and
      Grimm stepping out of it; it waiting through visitors / enemies and him coming back.
- [ ] Grimm's friendly visit: entrance flash + cape flourish (`TeleToIdle`), the bow and his
      line, the teleports around the knight (flashes, `LookBack` / `Hand Click` poses), the
      knight's startle / bow / glances, farewell; the flame left behind (8+ achievements).
- [ ] Hornet's race:
      - the needle (`Needle` + `Needle Thread` sprites) in flight and stuck on the finish
        ledge: drawn horizontal whatever the angle; its height above the ledge;
      - her course: runs, the ground dash, leaps up to windows (through them), the air dash
        on long leaps, the landing; her pace (slowed up to ~2x to match the knight);
      - the knight on "Go!" (Hurry route, crystal dash, wall jumps), touching the needle;
      - her win `Flourish` / lose line, plucking the needle, the leap away.
- [ ] Hornet's duel: lunges, needle throws (needle sprite out and back), the silk sphere
      (`Sphere Ball`), step-backs, her flashes when struck, the knight dodging / pogoing;
      the `Flourish` and her parting word.

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
