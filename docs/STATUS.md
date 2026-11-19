| Bug-test round 1 (day 2) | Sit loop broken (boredom drives exploration and world events); planner sees every wall again, follower re-plans after 0.5 s of pushing, auto-dives only on straight planned drops; Geo noticed desktop-wide; **step behind drawn lines** (TurnToBG / TurnFromBG) instead of dashing through them; **pester** mode (hops, loops, dash-bys, flicks around the cursor); repeated poses fuse instead of stuttering; thought cloud behind Geo / ? emotes; minimized windows shatter into Geo (once per window per 10 min) | #51 #52 #53 #54 #55 #56 #57 |
# Desk-Skong — status & roadmap

_Last updated 2026-11-19 (afternoon): bug-test round 1 fixed and merged (#51–#58); no open PRs._

## Done (merged to `master`)

| Area | What | PRs |
|---|---|---|
| Assets | Exporter CLI: tk2d collections as raw quads, animation libraries (207 Knight clips), vanilla atlases as a skin + fallback, Unity sprite export (benches) incl. SpriteAtlas, prefab hierarchy dump (effect offsets), `--dump-mono` (HeroController constants), `--find` | #7 #16 #19 #27 |
| Rendering | Rust rewrite; sprite-sized layered windows (0.2–1 % CPU), clip player with all tk2d wrap modes, tray with skin picker / size / mischief, settings persistence, control panel + log streaming + debug overlay (terrain + nav graph + route), panel opens on start | #8 #10 #17 #30 #34 |
| Terrain | Window platform boxes with occlusion; event-driven desktop watcher; live **content terrain** via DXGI duplication (dark-theme-sensitive ledges + content walls, activity heat map, ≈1 % CPU); frozen under our own windows; no walls on monitor seams | #12 #24 #33 #35 #37 |
| Movement | Controller with HK's real constants and full kit; hold-down drop-through; walking acts turn at walls/ledge ends; transitions, down-dash, Desolate Dive / Descending Dark, super hard landing, wall slash; hero effects (crystal dash charge/burst/trail/break, slashes, wings, puffs, dive) | #13 #23 #28 #29 #39 |
| Navigation | Surface graph validated by simulation, A*, travel styles, follower with re-planning, background incremental builds, go-to-cursor (Ctrl+Alt+G walk / H hurry) | #15 #25 #26 #34 #35 |
| Brain | Needs/emotions/personality, interest map, purposeful behaviours (companion, watch screen activity, explore & map windows, rest/nap/perch, battle, flee, hide, ride, recover), contextual acts, anti-ping-pong (reachable seats, failure cooldowns, oscillation guard), night lantern | #11 #20 #26 #31 #32 #38 |
| Props & world | Benches (tray, Ctrl+Alt+B), perching on any window/content edge, seated pose tuned; discoveries in windows (benches, geo deposits, per app, persisted) | #18 #21 #41 |
| Stakes | Death, the Shade (persisted, clamped, fights back), respawn, Geo coins + wallet (tray tooltip), shattered closed windows, Geo showers, Loot behaviour, spells & nail arts, panel "Stakes" buttons | #40 |
| Living world (day 2) | NPC visitors (Cornifer, Quirrel, Zote, Elderbug, Hornet) with original lines, speech/thought bubbles, subtle dust-puff portals, Dream Nail on windows/visitors (never mid-talk); discoveries v2 with shared occupancy (no overlaps; coins hop out of props), no hot springs, finds on platforms that lasted ≥ 30 s | #42 #46 |
| Training (day 2) | **Great Nailsage Sly** visits after the knight has fought (rarer than the others), talks, then runs a short lesson: he draws his nail, announces and demonstrates drills with his own clips (data-driven `demos`), the knight practises (`Train` behaviour: slashes / up / down slashes, jumps, dashes, Great / Cyclone / Dash Slash), Sly praises or corrects, closes and bows out; ends politely when the user types, leaves, or a fullscreen app comes up; mid-air slashes now render | #58 |
| Behaviour (day 2) | Priority tiers (Urgent > Engagement: Shade, Geo, fights, NPC talks > Needs > Ambient), graceful exits (get up, put the map away); seated life (calm breathing idle, map checks, leaning, dozing; minimum holds only where they make sense; mood-driven pace; all seated frames on one seat line); silent knight emote icons (zzz, Geo, !, ?) | #44 #45 #48 |
| Climbing (day 2) | Explicit wall-climb planning along any wall, shafts, content walls, monitor seams; robust replays and slip memory; 100 % arrival in 300-desktop soak | #43 |
| Input & safety | Grab-and-throw, clicks/pokes, idle & typing-rate sensing (counts only), single instance, **fullscreen games**: knight moves to a free monitor or vanishes (no CPU) | #22 #26 #36 |

## In flight

Nothing — all draft PRs from day 1 were finished and merged on day 2.

## Backlog (agreed ideas, not started)

- Crystal dash in the planner beyond gap crossing (long flat runs, wall-launched).
- Geo showers through the (subtle) portal; tray/panel trigger for visits.
- Ability progression (earn dash/wings/claw/crystal heart with the game's Collect
  cutscenes) — optional, not decided.
- Enemies / training targets wandering in from screen edges (NPC/enemy art from level
  files); Zote as a sparring partner.
- Hazards: `Spike Death` / `Acid Death` acts exist, nothing triggers them yet.
- Unn slug form (28 unused `Slug *` clips) for very calm moods.
- Dream Gate travel as the fallback when no route exists.
- HK particle systems (crystal shards, run/land dust) recreated procedurally.
- Lazy per-clip rasterization to cut frame memory at large scales.

## Design principles (from the user)

- Behaviour must have **reasons**; no random emote spam; balanced idle vs. meaningful holds.
- Props and visitors appear **organically** (found in windows, arriving from edges/portals),
  arrivals **subtle** (dust, fades — not flashy).
- High-value activities beat lounging (Shade, NPCs, Geo, fights > sitting).
- The knight is **silent**; polite in conversation (no Dream Nail while someone talks).
- Stay out of fullscreen **games**; other fullscreen apps are fine.
- Manual control is a toggle only (off by default); never leak keys to other apps.
- Agents testing live must never move the user's cursor or windows.

## Known issues

- Not yet seen live (only tested in simulation): step-behind crossings, pester, Sly's lesson (demo
  arcs vs. nearby windows, his nail vanishing at the bow: no sheathe clip), thought clouds.
- Training drills only check room on the current ledge; a dash slash near an edge may drop him
  off (he walks back).
- If the brain switches behaviour right after he sits (e.g. idle → companion), he may get
  up and sit straight back down (the new behaviour re-requests a seated act). Fix: keep a
  running seated act when the next behaviour wants to sit too.
- Nav graph builds are 26–72 % slower than before climbing on a 30-window stress desktop
  (background thread; ~3 ms median on realistic desktops). Next lever: share climb outcomes
  per source/wall across targets.
- Under `--capturable` (debug), vision can see our own effects/props.
- Rotated monitors get no content terrain.
