# Desk-Skong — status & roadmap

_Last updated 2026-11-19 (end of the revival session)._

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
| Input & safety | Grab-and-throw, clicks/pokes, idle & typing-rate sensing (counts only), single instance, **fullscreen games**: knight moves to a free monitor or vanishes (no CPU) | #22 #26 #36 |

## In flight (parked as draft PRs — see each PR for remaining steps)

| Branch | Goal | PR |
|---|---|---|
| `path_wall-climbing` | Explicit wall-climb planning along any wall (single wall, shafts, content walls, monitor seam), perturbed-start validation + backup climbs, follower retries/avoids failing hops, auto-dive gated during climbs, overlay draws climbs. Soak: 100 % of planned routes arrive. **Left:** the last pruning step regressed coverage (unreachable window tops 12→18 at scale 0.6, 7→23 at 1.0) — bisect its three changes; final timings | [#43](https://github.com/Amri-Tarik/Desk-Skong/pull/43) (draft) |
| `npc_visitors` | 5 visitors (Cornifer, Quirrel, Zote, Elderbug, Hornet) with original lines, speech/thought bubbles, `Greet` behaviour, subtle dust-puff portals (Geo showers use them), Dream Nail on windows and (after a talk) on visitors. Live-tested. Left: map `Greet` → Engagement tier, use shared occupancy, rethink the knight's own thought bubbles under the "silent knight" rule | [#42](https://github.com/Amri-Tarik/Desk-Skong/pull/42) (draft) |
| `ai_discoveries-v2` | Shared `skong_core::occupancy` (claim/release/is-free/nudge; finds slide or hide on collision, benches outrank deposits), hot springs removed (old saves load fine), finds only on ledges that lasted ≥ 30 s (width-weighted, incl. content lines). Tests pass; not live-checked. **Left:** hook Geo coin settling and tray-bench placement into occupancy, NPC/Shade claims, screenshot check | [#46](https://github.com/Amri-Tarik/Desk-Skong/pull/46) (draft) |
| `anim_seated-life` | Seated "life" state machine (idle, glance, look down, lean, map check, nod, doze → wake) via act step `{"life": "seated"}`; 8 new composite clips (`Knight.extra.json`: Sit Glance, Sit Lean Idle, Sitting Asleep Breathe, Prostrate Breathe, Look At King Idle…); `rest`/`lean_back` use it. Builds, tests pass. **Left:** `nap`/`read_map` on it, `bow_down`/`gaze_into_screen` → breathe/idle clips, pass mood from the app, state-machine tests, 30 s screenshot check, merge master | [#45](https://github.com/Amri-Tarik/Desk-Skong/pull/45) (draft) |
| `ai_priority-tiers` | Behaviour tiers: Urgent > Engagement (Shade, Geo, fights; NPC `Greet` is a one-line addition) > Needs (only when critical) > Ambient (sitting…), immediate preemption, `Intent.exit_act` for graceful exits. **WIP — 9 ai tests fail**: Battle's 0.25 "invite" baseline makes Engagement almost always top-tier (make Battle eligible only when actually invited / raise the engagement eligibility threshold). Left: exit acts in acts.json, app handling of `exit_act`, tier tests; not yet merged with master | [#44](https://github.com/Amri-Tarik/Desk-Skong/pull/44) (draft) |

Suggested order tomorrow (each needs `git merge origin/master` first):
1. **#45 seated life** and **#46 discoveries v2** — finish their small leftovers, merge.
2. **#42 NPC visitors** — adopt occupancy, merge.
3. **#44 priority tiers** — fix the Battle-eligibility test failures, add exit acts + app
   handling, map `Greet` to Engagement, merge.
4. **#43 wall climbing** — bisect the coverage regression, record timings, merge.
5. Backlog: knight emote icons first.

Worktrees for these branches live in `E:/Data/Desk-Skong-wt/<branch>`.

## Backlog (agreed ideas, not started)

- **Knight emote icons instead of speech** — he's silent in-game: zzz (tired), Geo icon
  (greed/excitement), "!" (surprise), "?" (curious); NPCs keep speech bubbles.
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

- Wall scaling is unreliable on tall single walls/shafts until `path_wall-climbing` lands.
- Under `--capturable` (debug), vision can see our own effects/props.
- Rotated monitors get no content terrain.
