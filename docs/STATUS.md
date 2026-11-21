# Desk-Skong — status & roadmap

_Last updated 2026-11-19 (night): everything through #95 merged — see the ROADMAP's
"Recently done" for the night batch (enemies, Dream Gate, regional rocks, slug, particles,
highlighted text, wall crystal dash, packed frames, faster nav builds); no open PRs._

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
| Secret boss | **The Troupe Master** (`data/grimm.json`): once 8 achievements are found, a dim scarlet flame now and then appears on a settled, reachable ledge; striking it brings Grimm out with a bow and a line (original writing). Teleports, dive + ground dash, slashes + uppercut, firebats (high / low), flame pillars, spikes, the balloon at each phase, all placed relative to the knight's ledge (any desktop is the stage). The knight answers the bow, then reads every tell (dodges, jumps spikes and bats, steps off pillars, keeps clear of the balloon) and uses his kit; lethal (a Shade), won ~85 % of seeded bouts; a big Geo burst and 2 achievements. Bows out on fullscreen apps, visitors, hard typing; vanishes for games. Debug: panel → Achievements → Debug. | (branch `npc_feat-grimm`) |
| Pastimes (day 3) | Less plain company (~91 % → ~70 % in a simulated half hour): after a while keeping company he takes up a **pastime** (`ai::pastime`, Ambient): practising on a **training dummy** that now and then stands on a settled ledge (combos, up slashes, pogos, nail arts, fireballs; it swings per blow), **chasing a lumafly** drifting over the windows (follows, jumps, sometimes catches it), **taking in the view** from the end of a high window edge, **stargazing** at night. Dummy art: `--files sharedassets321.assets --animations "Training Dummy Real Anim" --textures ../skins/Vanilla` (metadata in `data/hk`; a drawn stand-in without the art). Lumafly art: `--files sharedassets6.assets --animations "Glow Fly"` (collection `Ambient Life`; the procedural glow without it). | (branch `ai_feat-pastimes`) |
| Bug-test rounds 1–2 (day 2) | Sit loop broken (boredom drives exploration and world events); planner sees every wall again, follower re-plans after 0.5 s of pushing, auto-dives only on straight planned drops; the ledge he stands on always stays in the planner (no more stranding on short content ledges); wall climbs no longer trip the dithering guard; Geo noticed desktop-wide; **step behind drawn lines** (TurnToBG / TurnFromBG) instead of dashing through them; **pester** mode (hops, loops, dash-bys, flicks around the cursor); he falls off moved windows instead of riding them; repeated poses fuse instead of stuttering; thought cloud behind Geo / ? emotes; minimized windows shatter into Geo (once per window per 10 min) | #51–#57 #59–#61 |

## In flight

Nothing — all draft PRs from day 1 were finished and merged on day 2.

## Backlog

The prioritised backlog lives in [ROADMAP.md](ROADMAP.md) (the user edits and annotates it).

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
- Nav graph builds: ~2.0 ms median, ~4.2 ms max on the 8-window soak desktops, 5–13 ms on
  30-window stress desktops (background thread), since collision queries test geometry
  before the window-overlap rule (−15 % / −55 %, same graphs). Sharing climb simulations
  per source/wall across targets was measured: walls serve ~1.4 targets each on the soak
  desktops, so it saves ≤ 8 % of ticks there (≤ 18 % on stress desktops), less than a
  straightforward replay costs; not done.
- Under `--capturable` (debug), vision can see our own effects/props.
- Rotated monitors get no content terrain.
