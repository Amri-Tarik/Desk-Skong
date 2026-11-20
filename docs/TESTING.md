# Desk-Skong — manual test checklist

Grouped like the panel's **Sandbox** tab (its right half, the Test buttons). Each line:
**how to trigger it** → what should happen · *look for* what tests can't check. "On its
own" says when it happens without the panel. Tick the box, or add `> note: …` under a line.

**First, enable the sandbox.** In the Desk-Skong window, open the **🔒 Sandbox** tab, tick
"I understand…" and press **Enable sandbox** (or start the pet with `--sandbox`; the
forcing debug flags such as `--visit` turn it on too). A "Sandbox on — achievements
paused until restart" banner appears. Until then the pet refuses every button below (and
the K / A / R / D hotkeys, the tray's debug items): the footer says so.

**How the Test buttons work.** Every button acts now; the box at the top shows what came of
it (✔ / ✖ with the reason, e.g. "Myla can't sing by a Geo deposit: no Geo deposit on
screen"). **📍** buttons place the thing where you click next: the next left click
anywhere on screen is taken (it doesn't reach the app under it) and lands on the nearest
ledge at or below the click (title bar, content line, the floor), never on a fullscreen
game's monitor. A label by the cursor shows what you're placing; **Esc**, a **right-click**
or the panel's **Cancel** stops it; it gives up after 30 s. Clicks on the panel itself
still work while placing. Placed things skip the "lasting ledge" age rule for *where* they
may go, but on a window moved in the last ~15 s they only show once it has settled.

**Achievements:** in the sandbox nothing unlocks and no progress counts, and Geo collected
isn't saved, until the pet restarts. To check an achievement itself, restart without the
sandbox and make it happen naturally (or with the always-allowed tools: settings, placed
benches, walk / hurry to the cursor).

## The window (no sandbox needed)

- [ ] **Close (X)** → the pet quits too (knight, tray icon gone).
- [ ] **Hide to tray** → only the window goes; the knight stays; the hint line under the
      button says how to get it back. **Left-click the tray icon** → the window comes back.
- [ ] **Tray click with the window open but behind others** → it comes to the front (no
      second window). A quick double click → still one window.
- [ ] **Tray → Quit** or **Ctrl+Alt+Q** → the pet quits and the window closes with it.
- [ ] **Kill `skong-panel.exe`** (Task Manager) → the knight carries on; a tray click opens a
      new window.
- [ ] **Run `skong.exe` again** while it runs → no second knight; the window comes up.
- [ ] **Home**: the Journal (count, progress line, Geo wallet with the game's own Geo coin
      sprite + number, "Time with the knight  12h 34m") and The Knight (skin, size,
      mischief, the switches, benches) act at once.
- [ ] **The look** · *look for*: body text ~17 px and hints ~15 px, all easy to read (hints
      pale grey-white, not dim); Cinzel headings and tabs, Crimson Pro body; near-black
      blue-grey ground, square frames with thin pale borders and corner ticks, the drawn
      diamond dividers; no gold / green / saturated colours (only the sandbox warning is a
      muted ember). The default window fits everything; narrowed below ~900 px the two
      Home frames stack and nothing is cut off. Without extracted art (e.g. a dev build
      with no `skins/`) the Geo icon is a drawn pale stone, not a gold coin. The setup window
      (`skong-panel --setup`, or Re-extract game art…) has the same look.
- [ ] **Advanced** (bottom right) → the Diagnostics tab (status + logs) appears.
- [ ] **Locked sandbox**: the "Sandbox (locked)" tab explains the cost; Enable stays greyed until
      the box is ticked; Ctrl+Alt+K / A / R / D and panel letters K A R D do nothing (a log
      line / footer says why); the tray menu has no Sandbox submenu.

**General:** the knight never moves your cursor or windows; nothing of his shows on a
monitor running a fullscreen game; visitors / enemies / Grimm wait while you type hard or
a fullscreen app is up.

## Visitors

Pick a visitor in the drop-down, or use the "Every scene" row.

- [ ] **Visit (talk)** (any visitor) → a subtle dust portal (Hornet walks in), the knight
      runs over and they talk; nothing follows · *look for* the speech bubbles' pace and
      position, the knight's greeting act, them leaving through a portal afterwards.
- [ ] **Visit (natural)** → as above, the scene after the talk rolled as on a natural visit.
- [ ] **📍 next to a visit** → they arrive through a portal at the clicked spot (any
      arrival style) · *look for* the knight reaching them, or talking from a reachable spot
      within range (6 units across, 5 up / down) when he can't get beside them.
- [ ] **Talk from nearby**: put a visitor (📍) on a busy window edge he can't stand next to
      → he talks from the closest reachable spot, same level first.
- [ ] **Sly lesson** → Sly talks, draws his nail, announces and demonstrates drills with his
      own clips; the knight practises (slashes, up / down slashes, jumps, dashes, Great /
      Cyclone / Dash Slash); praise or corrections; the bow · *look for* demo arcs against
      nearby windows, his nail vanishing at the bow (no sheathe clip), a dash slash near a
      ledge end carrying the knight off (he should walk back). Achievement *Diligent
      Student* once seen to the final bow. On its own: only within 15 min after a fight,
      rarer than the others; always teaches.
- [ ] **Zote spar** → after the talk Zote challenges; they spar until Zote goes down or it
      fizzles · *look for* pogo chains off Zote, hit flashes, his lines. *Humbled the
      Mighty* if the knight wins. On its own: 60 % of Zote's talks.
- [ ] **Cornifer maps** → they sit side by side, both maps open, Cornifer humming (bubbles
      over his drawing idle); on the desktop floor they stand with the map · *look for* the
      gap between them, the hum bubbles' pace. *Comparing Notes*. On its own: 50 % of his
      talks.
- [ ] **Quirrel view** → Quirrel arrives on a high window edge (upper half of a monitor);
      both sit on it (Bench Look facing, Sit Lean pose), then Bench Look End · needs a
      settled high window edge (the result line says so when there's none, and it's just a
      talk). *Room for Two*. On its own: 55 % of his talks, only on such an edge.
- [ ] **Myla song** → Myla arrives beside a Geo deposit, mines facing the rock and sings ·
      needs a deposit on screen first (**Discoveries & Geo → Geo deposit**) · *look for*
      her size and feet line (new art), not overlapping the rock or the knight's mining
      spot, the song bubbles' pace. *Work Song*. On its own: 70 % of her talks, beside a
      deposit when there is one.
- [ ] **Calm scene cut short**: start any calm scene, then type fast / send an enemy / kill
      him → the visitor's abandon line, the knight leaves.
- [ ] **Hornet race** → she throws her needle (`Needle` + `Needle Thread`) to a finish
      ledge; "Go!"; they race across the windows; her win `Flourish` or lose line; she
      plucks the needle and leaps away · needs a settled finish on her monitor he can reach
      (else a duel, and the result line says so) · *look for* the needle drawn horizontal
      whatever its flight angle and its height above the ledge; her run, ground dash, leaps
      up to windows (through them), the air dash on long leaps; her pace (slowed up to ~2×
      to match him); the knight's Hurry route (crystal dash, wall jumps) and touching the
      needle. *Faster Than Silk* when he wins (about 1 race in 4).
- [ ] **Hornet duel** → lunges, needle throws (out and back), the silk sphere, step-backs,
      her flashes when struck, the knight dodging / pogoing; her `Flourish` and parting
      word. *Needle Practice*. On its own: 60 % of her talks end in a race or duel (50/50).
- [ ] **Challenge held**: type fast, or have the Shade waiting, then ask for a race → "no
      challenge now (…)", just a talk; mid-race, start typing → called off.
- [ ] **Elderbug / any visitor, Dream nail** → after the talk the knight may dream-nail
      them: their thought in a cloud bubble; never while someone is talking. *Uninvited,
      Inside*. **Dream nail** (button) on a window → a thought about that window. *Dreams of
      Glass*.
- [ ] **Every visitor met** → *Full Guestbook*; **Hornet talk** → *Silken Small Talk*.
- [ ] On its own: visits come now and then while you're around (not while typing hard,
      fullscreen, or with him asleep); not the same visitor twice in a row.

## Enemies

- [ ] **Crawlid / Vengefly / Wandering Husk** (one each) → it wanders in (walks in from a
      monitor edge, drops onto a ledge, or flies in) and the knight fights it · *look for*
      the result line naming where it came in.
- [ ] **📍 an enemy** → it drops in at the clicked spot (a flyer hovers above it).
- [ ] Combat kit (send a few, watch): **pogo chains** on enemies; **Desolate Dive** from a
      window top onto a foe below; **shadow-dash dodge through a Husk lunge**; a
      **crystal-dash opener**; **Howling Wraiths at a Vengefly**; Focus after a hurt fight.
- [ ] **Several at once** (3 quick clicks) → no walk-stop-walk stutter; a foe in his way
      becomes the target.
- [ ] **Geo after kills** → coins burst from each kill and he gathers them all (even after a
      rough fight; only sleep beats Geo).
- [ ] **Vengefly felled without touching the floor** → *The Floor Is Lava*; one of every
      kind → *Field Guide*.
- [ ] **Start a gauntlet** → at the next good moment: three waves dropping in through dust
      puffs, then a big Geo burst · *look for* wave pacing, the drop-in puffs, the reward
      burst, how often he finishes one; it ends early if you type / go fullscreen, and
      never starts with the Shade waiting. *Ran the Gauntlet*.
- [ ] On its own: rare, organic arrivals; sometimes the bored knight asks for one.

## Grimm

- [ ] **Light a scarlet flame** (or **📍** it into a prepared arena) → a dim flame on a
      settled ledge; the knight lingers by it with a "?" every 7–13 s; **you click the
      flame** (only its window takes the click) → Grimm steps out of it with a bow and a
      line · *look for* the flame's flicker; it waiting ~30 min, through visitors / enemies,
      and the knight coming back to it; the summoned flame still winning over the dummy.
- [ ] **Summon the troupe master** → straight to the show beside the knight · *look for*
      Grimm's size, feet line and facing; teleport flashes; firebat / fireball / pillar
      scale and orientation; pillar glow and spikes; the balloon each phase; the knight
      dodging, jumping spikes / bats, stepping off pillars; who wins on a real desktop
      (~85 % for the knight in the sim). *Final Curtain* (win), *Taken a Bow* (loss: a
      Shade). The result line says what he waits for when it isn't a good moment.
- [ ] **Friendly visit** → entrance flash + cape flourish (`TeleToIdle`), the bow and his
      line, teleports around the knight (`LookBack` / `Hand Click` poses), the knight's
      startle / bow / glances, farewell; the flame left behind (8+ achievements only).
      *A Passing Troupe*.
- [ ] On its own: once 8 achievements are unlocked, a flame now and then; the friendly
      visit is rare. A fullscreen game: everything vanishes at once.

## Pastimes & props

- [ ] **Training dummy** (near him) or **📍** → it stands on the ledge, he practises at once
      (combos, up slashes, pogos, Great / Cyclone / Dash Slash, fireballs); it swings per
      blow · *look for* its size beside him (`DUMMY_ART_SCALE` 0.34), its foot on the
      ledge, the hit clips swinging *away* from the blow (L / R is a guess), slash / pogo
      reach, the dash slash not carrying him off, hit markers on the figure, a short white
      hit flash per blow; whether he runs off to it too eagerly.
- [ ] **Strike the dummy yourself**: click its art (press and let go without moving more
      than ~6 px) → it swings away from the side you clicked (`Light Hit L/R`), flashes
      white, the hit marker where you clicked; the log has "the user struck training dummy
      #… (N % worn)". About 5 clicks break a fresh one (fewer once worn) → it fades out fast
      in a burst of brown / beige splinters (+ a puff of dust at its foot); log "training
      dummy #… broke … (the user's clicks; the next one waits longer)". The next one comes
      2.5× later than usual. Press and move → still a drag, not a blow · *look for* the
      splinters' size / colour / spread, the fade's speed, whether ~5 clicks feels right.
- [ ] **The dummy breaking while he practises**: place one (📍) and click it 4 times, then let
      him practise → his blows finish it ("under his nail" in the log): the swing in progress
      finishes, he looks down at the pieces (`LookDown`) and goes back to what he was doing.
      First time → *Splinters*. Same for one broken by your click mid-session (no
      achievement).
- [ ] **Wear over time**: a dummy left standing breaks by itself after 13–18 min (sooner
      with practice: ≈ 15 % a session, ≈ 8–10 min of keen practice); placed (📍) and dragged
      ones wear the same.
- [ ] **Lumafly** → a glow drifts from title bar to title bar; he follows along the ledges,
      jumps at it, sometimes catches it (a flare) and watches it float off · *look for* the
      glow's size / colour / bob, how catchable it feels.
- [ ] **The view** → he climbs to the end of a high, settled window edge and sits taking in
      the view (or the result line says there's no such edge) · *look for* which end he
      picks and which way he faces.
- [ ] **Stargazing** (any hour from here) → up to an open ledge, Look At King held 16–28 s.
- [ ] On its own: the dummy 4–10 min after the last one went (it stands until it breaks),
      lumaflies now and then; the
      view and the stars after ~25 s of plain company or when bored (stars 21:00–05:00);
      *look for* how often versus keeping company (aim: plain company ~70 % of the time).

## Discoveries & Geo

- [ ] **Bench in his window** → a found bench appears in the window he stands on; he may
      sit; seated life (breathing idle, map checks, leaning, dozing). Every kind sat on →
      *Connoisseur of Seats*.
- [ ] **📍 Bench** (pick a style) → a placed bench on the clicked ledge, riding its window.
- [ ] **Geo deposit** (his window) or **📍** (click a window's title bar / a line in it) →
      the region's Geo Rock with its gleam; he mines it (3 hits, Geo out of the rock), and a
      new one turns up in another app later · *look for* the art and gleam for every region
      (the region follows the app: try several apps), sizes. Every region mined →
      *Rockhound*.
- [ ] **Shiny** or **📍** → a glint; he walks over and picks it up (`Collect Normal 2`),
      sometimes Geo pops out · *look for* the twinkle and the pick-up timing.
- [ ] **Geo shower** or **📍** → a subtle dust portal, coins tumble out a moment later; he
      gathers them. On its own: when he's bored, over settled, reachable ledges.
- [ ] **Spill Geo at the cursor** → 30 Geo burst from the cursor; he collects; a big haul
      (40+ in 12 s) → a sparkle bubble. 1000 picked up → *Hoarder*.
- [ ] **Close a window** → it shatters into Geo (minimized windows too, once per window per
      10 min). *Glass into Geo*.
- [ ] On its own: exploring a new app's window may reveal a bench or deposit (persisted);
      shinies now and then on lasting ledges.

## Dragging props & visitors

Draggable: the training dummy, visitors, Geo deposits, benches (placed and found). Not:
enemies, Grimm and his flames (the scarlet flame's click still summons him), the Shade,
Geo coins, shinies, lumaflies. Press on one and move a few px → picked up; let go → it
falls onto the first ledge below that's wide enough for it (a title bar, the floor, or a
content line that has settled); over a fullscreen game's monitor it drops onto the nearest
other monitor instead. A plain click does nothing (on the dummy it's a blow: see
"Strike the dummy yourself" above).

- [ ] **Hit-testing**: click a *transparent* part of a prop's / visitor's box (beside the
      dummy's pole, around a bench's legs) → the click reaches the window under it; click
      the art itself → no click on the app below (the dummy swings) · *look for* soft edges
      (semi-transparent pixels catch the mouse too), the cursor never moving.
- [ ] **Training dummy**: drag it (it wobbles when lifted, thuds on landing) onto another
      window / the floor → it stays there, never moved away by itself; the log has "training
      dummy #… stays where it was dropped; he'll go and practise there" or "… he can't
      practise there …", and the brain's own verdict line again · *look for* the grab offset
      (it hangs from where you took it), the fall speed, the foot on the ledge.
- [ ] **Visitor, waiting**: pick them up → a surprised line in their voice, the knight's
      "!" (and he turns to look when free); drop them elsewhere → they wait, he runs over
      and they talk as usual.
- [ ] **Visitor, mid-talk**: pick them up during the 2nd line → the bubble closes, their
      surprised line; set down → he comes over again (or talks from nearby) and the talk
      goes on from the line it stopped at (no fresh conversation).
- [ ] **Visitor, mid-lesson / spar / calm scene / Hornet's race or duel**: pick them up →
      the scene ends at once (no stuck knight, no needle left behind); set down → a parting
      line ("set_down" in `data/npcs.json`), then they leave as usual · *look for* the
      knight going back to his own business, Zote's held-back goodbye not said.
- [ ] **Geo deposit**: half-mine one (1–2 hits), drag it onto another app's window → it
      rides that window and takes that app's region's rock look; the remaining hits are
      kept; it's still there after a restart (`world.json`). Onto the floor → it lies there
      (same look), whatever windows open or close. Never on a fullscreen window / the pet's
      own windows (it falls past them).
- [ ] **Benches**: drag a placed bench (📍 Bench) or a found one while he **sits** on it →
      he gets up first; the bench lands and rides its new window; he may come back and sit.
- [ ] **Interior Decorator**: five different things dragged and set down → unlocked.

## Knight

- [ ] **Kill him** → death, a Shade where he fell holding his Geo, respawn; ~4 s later he
      goes after it (a Dream Gate if no route reaches it); the Shade fights back · *look
      for* no minutes-long delay, no giving up. Fall to death then reclaim → *What Goes
      Down*.
- [ ] **Hunt the Shade now** → he heads for it at once (the result line says when there's
      none).
- [ ] **Fatal fall**: drag him high and drop him onto the floor of a tall monitor → a super
      hard landing, or death → Shade. *Long Way Down*.
- [ ] **Spells** (each button) → Vengeful Spirit (fireball), Howling Wraiths, Great Slash,
      Cyclone Slash, Dash Slash, Thorns, Focus · *look for* the effects lining up with him.
      *Soul Spent* comes from a Vengeful Spirit / Howling Wraiths he casts on his own (in a
      fight); the panel's spell buttons only play the act and don't count.
- [ ] **Slug form** → Unn's slug crawl along the ledge, turning at its ends; he gets up when
      something comes up. *Slow and Serene*. On its own: when serene, now and then.
- [ ] **Emote cycle** (on / off) → every bubble above his head, 2.5 s each · *look for* at
      every Size: the anger mark (four arcs), the heart, the sweat drop sliding down, the
      "…" cloud, "!", "?", "z" / "zzz", the Geo thought cloud; colours on light and dark
      windows; the four-step fade in / out. On its own: rare (at most ~1 a minute) — note
      how often over a real day; "z" when settling for a nap vs "zzz" asleep.
- [ ] **Dream Gate → Ready**, then put a prize out of reach, or **📍 Target** on a window top
      with nothing to climb → he plants a gate there and warps · *look for* the gate's
      size, glow and warp timing. *Shortcut Through Sleep*. On its own: for an unreachable
      visitor / Geo / deposit / shiny, at most every 90 s.
- [ ] **Preview achievement banner** → near the top centre of the knight's monitor:
      "Achievement" small, your latest achievement's title large in Cinzel, the diamond
      divider, the description (at most two lines) over a soft dark mist · *look for*: fades
      in ~0.6 s, holds ~4 s, fades out ~1 s; click several times → they play one after
      another; readable on light and dark wallpapers and at 100 % / 150 % scaling; clicks
      pass through it and it never takes focus (keep typing in another app while it shows);
      hidden from screenshots / OBS unless "Visible in screen shares" is on; with a
      fullscreen game or video on the knight's monitor it shows on another monitor, or
      waits until the fullscreen app is gone (and vanishes at once if one starts mid-way,
      then replays); Pause hides it and it replays after. On its own: every real unlock
      shows one (the sparkle above his head and the tray tip stay too).

## Everyday behaviour (no button)

- [ ] **Carried map**: idle / keeping company, the map comes out about every 2 min for 15–40 s
      (Map Open → Map Idle, Map Walk while following the cursor, Map Turn when turning in
      place, Map Away → Idle without a snap); folded before a fight, hurrying or sitting;
      dropped mid-jump.
- [ ] **Carried lantern** (setting "Lantern on dark backgrounds", on by default): over a
      dark-mode window or a dark wallpaper, Lantern Idle / Lantern Run while he follows the
      cursor; it pops off over acts and jumps; with the setting off it never comes out
      (tray, panel) · *look for* the dark-detection thresholds and whether it being out the
      whole time on a dark desktop feels right.
- [ ] **Highlighted text** (on by default) → he notices a selection and goes to look at it.
      *Literate Bug*.
- [ ] **Step behind drawn lines** (TurnToBG / TurnFromBG) instead of dashing through them.
      *Behind the Lines*.
- [ ] **Pester** (Mischief High) → hops, loops, dash-bys, flicks around the cursor for 45–90 s.
- [ ] **Fight the cursor** (toggle) → rounds of battle; hit markers on the cursor and the
      defeat burst. *Bested by an Arrow*, *Pointer Collector* (5 wins).
- [ ] **Picking him up** → drag and throw (*Bug Toss*), catch him mid-air (*Nice Catch*),
      poke him napping (*Do Not Disturb*); a window he stands on dragged away → he falls off
      (*Unscheduled Departure*).
- [ ] **Wall climbs**: five wall jumps in a row (*Up the Shaft*); a crystal dash charged on a
      wall (*Off the Wall*).
- [ ] **Thought cloud** behind the Geo / "?" emotes; seated map open / read / close.
- [ ] **Screen-share toggle** ("Visible in screen shares") → he shows in OBS / Discord
      captures while on, not while off.
- [ ] **Fullscreen game** on his monitor → he dream-gates to a free monitor or hides; props,
      visitors, enemies and Grimm leave that monitor.
