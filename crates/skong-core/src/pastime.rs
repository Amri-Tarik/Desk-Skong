//! Pastime props: things the knight amuses himself with when nothing better is going on
//! (the brain side is `ai::pastime`).
//!
//! * **The training dummy** ([`Yard`]): now and then a dummy stands on a settled ledge (the
//!   shinies' placement rules: lasting surfaces, shared occupancy, never on a game monitor,
//!   reachable, not right under his feet). Struck, it swings (`Light Hit` / `Heavy Hit`, L
//!   or R by the side the blow comes from). It wears out: standing (≈ 13–18 min alone), his
//!   blows (a practice session takes ≈ a fifth of it, never all) and the user's clicks (≈ 5
//!   break a fresh one) all wear it, and at nothing left it breaks ([`DummyBreak`]): a quick
//!   fade while the app throws splinters, then the next one on the usual schedule (later
//!   when the user broke it: they didn't want it there).
//! * **A lumafly** ([`Sky`]): a small glowing fly that drifts from window to window (a
//!   little above their title bars) on the knight's monitor, bobbing. Swiped at, it may be
//!   caught (it glows bright and slips away upwards) or it darts off. It leaves after a
//!   minute or two.
//!
//! Pure: the app ticks, places and draws them (`glow` for the lumafly).
use crate::canvas::{Image, premul};
use crate::desktop::DesktopSnapshot;
use crate::discovery::Mix;
use crate::geom::{RectF, Vec2};
use crate::shiny::{self, ShinySite, Spot};
use crate::terrain::{Ledge, LedgeKind, Terrain};

// --- the training dummy ---------------------------------------------------------------

/// How often a dummy stands somewhere, and how it wears out.
///
/// Wear is a share of the dummy (1 = broken), summed over time and blows. A drill wears
/// ≈ 3 % on average (a combo's three slashes 3.6 %, a nail art or fireball 3 %, a cyclone
/// 5.4 %), a session of 4–6 drills ≈ 15 % (at most ≈ 27 %: one never breaks a fresh dummy).
/// Practising every ≈ 3 min (the pastime's cooldown plus the session), it breaks after
/// ≈ 8–10 min; with no practice after ≈ 13–18 min; ≈ 5 clicks break a fresh one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DummyRules {
    /// Seconds between dummies, [min, max], counted from when the last one went (only
    /// counts while allowed).
    pub every_secs: [f32; 2],
    /// Seconds standing alone wears one out, [min, max] (rolled per dummy).
    pub life_secs: [f32; 2],
    /// Never closer to the knight than this (px at scale 1).
    pub min_distance: f32,
    /// Wear of one light blow (a nail slash, a pogo) and one heavy (nail arts, spells).
    pub light_wear: f32,
    pub heavy_wear: f32,
    /// Wear of one click by the user.
    pub click_wear: f32,
    /// The next dummy after one the user broke waits this many times longer.
    pub user_break_delay: f32,
}

impl Default for DummyRules {
    fn default() -> Self {
        Self { every_secs: [240.0, 600.0], life_secs: [780.0, 1080.0], min_distance: 220.0, light_wear: 0.012, heavy_wear: 0.03, click_wear: 0.2, user_break_delay: 2.5 }
    }
}

/// Fade in / out (s).
pub const DUMMY_FADE_IN: f32 = 1.2;
pub const DUMMY_FADE_OUT: f32 = 2.5;
/// A broken dummy fades this fast (s) while its splinters fly.
pub const DUMMY_BREAK_FADE: f32 = 0.35;

/// What broke the dummy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Breaker {
    /// Standing out in the weather (time).
    Wear,
    /// His blow (practice).
    Knight,
    /// The user's click.
    User,
}

/// A dummy broke (see [`Yard::take_broken`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DummyBreak {
    pub id: u64,
    pub by: Breaker,
}

/// A blow landing on the dummy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DummyHit {
    /// Nail arts and spells; nail slashes are light.
    pub heavy: bool,
    /// The blow comes from his right (the knight stands right of it).
    pub from_right: bool,
}

impl DummyHit {
    /// The clip of `Training Dummy Real Anim` it plays (knocked away from the blow).
    pub fn clip(self) -> &'static str {
        match (self.heavy, self.from_right) {
            (false, false) => "Light Hit R",
            (false, true) => "Light Hit L",
            (true, false) => "Heavy Hit R",
            (true, true) => "Heavy Hit L",
        }
    }

    /// The user's click at `at` on a dummy standing at `foot`: a light blow from that side
    /// (clicked right of the pole, it swings left).
    pub fn clicked(at: Vec2, foot: Vec2) -> DummyHit {
        DummyHit { heavy: false, from_right: at.x > foot.x }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dummy {
    pub id: u64,
    pub spot: Spot,
    /// [`Yard::clock`] when it turned up and when it starts fading away (never, unless sent
    /// away: it breaks instead).
    pub born: f64,
    pub leave_at: f64,
    /// The user put it there (the Test tab placed it, or it was dragged): it stays, even
    /// where he can't get to it (see [`Yard::relocatable`]).
    pub pinned: bool,
    /// Seconds standing alone would wear it out.
    pub life: f32,
    /// Wear from blows and clicks (time's share is [`Dummy::wear_at`]'s).
    pub knocks: f32,
    /// When it broke (it fades out [`DUMMY_BREAK_FADE`] s, then it's gone).
    pub broke_at: Option<f64>,
}

impl Dummy {
    /// How worn it is at `clock` (0 fresh, 1 and over: broken).
    pub fn wear_at(&self, clock: f64) -> f32 {
        ((clock - self.born) as f32 / self.life.max(1.0)).max(0.0) + self.knocks
    }

    pub fn is_broken(&self) -> bool {
        self.broke_at.is_some()
    }
}

/// The dummy on screen this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShownDummy {
    pub id: u64,
    /// Bottom centre (where the pole meets the ledge).
    pub pos: Vec2,
    /// Its ledge (x0, x1).
    pub span: (f32, f32),
    /// 0..1 (fading in / out).
    pub alpha: f32,
    /// Broken: fading out fast (no blows, clicks or drags; the brain doesn't see it).
    pub broken: bool,
}

/// The training dummy (at most one) and when the next one turns up.
#[derive(Clone, Debug)]
pub struct Yard {
    dummy: Option<Dummy>,
    /// Blows on their way (clock time they land).
    pending: Vec<(f64, DummyHit)>,
    /// Blows that landed since the app last looked.
    landed: Vec<DummyHit>,
    /// Dummies that broke since the app last looked.
    broken: Vec<DummyBreak>,
    next_id: u64,
    clock: f64,
    next_at: f64,
    /// The dummy went: the next one is scheduled from then, its delay times this.
    reschedule: Option<f32>,
    mix: Mix,
}

/// Seed tag ("dumy").
const DUMMY_TAG: u64 = 0x6475_6d79;

impl Yard {
    pub fn new(seed: u64, rules: &DummyRules) -> Self {
        let mut y = Self { dummy: None, pending: Vec::new(), landed: Vec::new(), broken: Vec::new(), next_id: 1, clock: 0.0, next_at: 0.0, reschedule: None, mix: Mix(seed ^ DUMMY_TAG) };
        y.schedule(rules, 1.0);
        y
    }

    fn schedule(&mut self, rules: &DummyRules, times: f32) {
        let [lo, hi] = rules.every_secs;
        self.next_at = self.clock + ((lo + (hi - lo).max(0.0) * self.mix.f32()) * times.max(0.0)) as f64;
    }

    /// The dummy is gone (faded, its window closed, broken): the next one counts from now.
    fn remove(&mut self) {
        self.dummy = None;
        self.pending.clear();
        self.reschedule.get_or_insert(1.0);
    }

    /// Wear it by `amount`; at 1 it breaks (`by`). `true` if it broke.
    fn knock(&mut self, amount: f32, by: Breaker, rules: &DummyRules) -> bool {
        let clock = self.clock;
        let Some(d) = self.dummy.as_mut().filter(|d| !d.is_broken()) else { return false };
        d.knocks += amount;
        if d.wear_at(clock) < 1.0 - 1e-4 {
            return false;
        }
        d.broke_at = Some(clock);
        let id = d.id;
        self.pending.clear();
        self.broken.push(DummyBreak { id, by });
        self.reschedule = Some(if by == Breaker::User { rules.user_break_delay } else { 1.0 });
        true
    }

    pub fn clock(&self) -> f64 {
        self.clock
    }

    pub fn dummy(&self) -> Option<&Dummy> {
        self.dummy.as_ref()
    }

    /// Advance the clock, land due blows and wear the dummy (it may break: see
    /// [`Self::take_broken`]). `true` when a dummy should turn up now (then [`place`] one, or
    /// [`Self::retry_soon`]). The timer only runs while `allowed`.
    pub fn due(&mut self, dt: f32, allowed: bool, rules: &DummyRules) -> bool {
        self.clock += dt as f64;
        let now = self.clock;
        let (due, later): (Vec<_>, Vec<_>) = self.pending.drain(..).partition(|(t, _)| *t <= now);
        self.pending = later;
        for (_, hit) in due {
            if !self.dummy.is_some_and(|d| !d.is_broken()) {
                break;
            }
            self.landed.push(hit);
            let w = if hit.heavy { rules.heavy_wear } else { rules.light_wear };
            self.knock(w, Breaker::Knight, rules);
        }
        // Time's share (no knock): broken once it adds up.
        self.knock(0.0, Breaker::Wear, rules);
        if self.dummy.is_some() {
            return false;
        }
        if let Some(times) = self.reschedule.take() {
            self.schedule(rules, times);
        }
        if !allowed {
            self.next_at += dt as f64;
            return false;
        }
        if self.clock < self.next_at {
            return false;
        }
        self.schedule(rules, 1.0);
        true
    }

    /// The user clicked dummy `id`: it wears ([`DummyRules::click_wear`]); `Some(true)` if
    /// that broke it, `None` if there's no such dummy standing. Not practice: not among
    /// [`Self::take_landed`] (the app swings it at once), and the brain never hears of it.
    pub fn click(&mut self, id: u64, rules: &DummyRules) -> Option<bool> {
        if !self.dummy.is_some_and(|d| d.id == id && !d.is_broken()) {
            return None;
        }
        Some(self.knock(rules.click_wear, Breaker::User, rules))
    }

    /// Dummies that broke since the last call (the app throws the splinters).
    pub fn take_broken(&mut self) -> Vec<DummyBreak> {
        std::mem::take(&mut self.broken)
    }

    /// How worn the dummy standing is (0 fresh .. 1 broken), for the log.
    pub fn wear(&self) -> Option<f32> {
        self.dummy.map(|d| d.wear_at(self.clock).min(1.0))
    }

    /// No room just now: try again in a little while.
    pub fn retry_soon(&mut self) {
        self.next_at = self.clock + 30.0;
    }

    /// A dummy now stands at `spot`.
    pub fn add(&mut self, spot: Spot, rules: &DummyRules) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let [lo, hi] = rules.life_secs;
        let life = lo + (hi - lo).max(0.0) * self.mix.f32();
        self.dummy = Some(Dummy { id, spot, born: self.clock, leave_at: f64::INFINITY, pinned: false, life, knocks: 0.0, broke_at: None });
        self.pending.clear();
        self.landed.clear();
        self.reschedule = None;
        id
    }

    /// The user put dummy `id` where it stands (the Test tab): it stays there.
    pub fn pin(&mut self, id: u64) {
        if let Some(d) = self.dummy.as_mut().filter(|d| d.id == id) {
            d.pinned = true;
        }
    }

    /// The user dragged dummy `id` to `spot`: same dummy (and stay), new place, pinned there.
    /// Blows on their way miss. Wear goes with it. Not a broken one.
    pub fn moved(&mut self, id: u64, spot: Spot) -> bool {
        let Some(d) = self.dummy.as_mut().filter(|d| d.id == id && !d.is_broken()) else { return false };
        d.spot = spot;
        d.pinned = true;
        self.pending.clear();
        true
    }

    /// May dummy `id` be moved elsewhere (or sent away) when he can't get to it? Not one the
    /// user put where it is.
    pub fn relocatable(&self, id: u64) -> bool {
        self.dummy.is_some_and(|d| d.id == id && !d.pinned && !d.is_broken())
    }

    /// A blow lands after `delay` seconds (the swing / the fireball's flight).
    pub fn strike(&mut self, hit: DummyHit, delay: f32) {
        if self.dummy.is_some_and(|d| !d.is_broken()) {
            self.pending.push((self.clock + delay.max(0.0) as f64, hit));
        }
    }

    /// Blows that landed since the last call (the app plays their clips).
    pub fn take_landed(&mut self) -> Vec<DummyHit> {
        std::mem::take(&mut self.landed)
    }

    /// Leave now (fade out): a game came up on its monitor, debug.
    pub fn dismiss(&mut self) {
        if let Some(d) = &mut self.dummy {
            d.leave_at = d.leave_at.min(self.clock);
        }
    }

    pub fn clear(&mut self) {
        self.dummy = None;
        self.pending.clear();
        self.landed.clear();
    }

    /// The dummy on screen (`None` if there is none or its ledge is covered right now). One
    /// sent away and faded, broken and faded, or whose window closed or went fullscreen, is
    /// gone (the next one is scheduled from then).
    pub fn shown(&mut self, desk: &DesktopSnapshot, terrain: &Terrain, width: f32, settled: &dyn Fn(&Ledge) -> bool) -> Option<ShownDummy> {
        let clock = self.clock;
        let d = self.dummy?;
        let open = match d.spot {
            Spot::Window { id, .. } => desk.window(id).is_some_and(|w| !w.fullscreen),
            Spot::Ground { .. } => true,
        };
        let faded = clock >= d.leave_at + DUMMY_FADE_OUT as f64 || d.broke_at.is_some_and(|t| clock >= t + DUMMY_BREAK_FADE as f64);
        if !open || faded {
            self.remove();
            return None;
        }
        let (pos, span) = shiny::resolve(terrain, d.spot, width, settled)?;
        let mut alpha = ((clock - d.born) as f32 / DUMMY_FADE_IN).min((d.leave_at - clock) as f32 / DUMMY_FADE_OUT + 1.0).clamp(0.0, 1.0);
        if let Some(t) = d.broke_at {
            alpha *= (1.0 - (clock - t) as f32 / DUMMY_BREAK_FADE).clamp(0.0, 1.0);
        }
        Some(ShownDummy { id: d.id, pos, span, alpha, broken: d.is_broken() })
    }
}

/// Why no dummy could be placed (for the log).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DummyMiss {
    /// Ledges with room for one (lasting, in the area).
    pub ledges: usize,
    /// Spots on them that are free, off game monitors and not under his feet.
    pub spots: usize,
    /// Of those, how many were route-planned (none had a stand spot he'd walk to).
    pub tried: usize,
}

impl std::fmt::Display for DummyMiss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ledges == 0 {
            write!(f, "no lasting ledge with room for one on his monitor")
        } else if self.spots == 0 {
            write!(f, "{} ledges, but every spot is taken, on a game monitor or too close to him", self.ledges)
        } else {
            write!(f, "{} free spots on {} ledges; none of the {} nearest has a stand spot his routes land on", self.spots, self.ledges, self.tried)
        }
    }
}

/// Most spots route-planned per placement, and per ledge.
const DUMMY_PLANS: usize = 16;
const DUMMY_PER_LEDGE: u8 = 3;
/// Spots are sampled this far apart along a ledge (px).
const DUMMY_STEP: f32 = 48.0;

/// Choose where a new training dummy stands: the shinies' rules (lasting ledges, free, off
/// game monitors, not right under his feet; `site.reachable` is not used) and, checked
/// last, `practisable(foot, ledge span)`: the brain's own test that he would walk over and
/// practise there (`ai::dummy_stand`), so the two always agree. Nearer and on his level (or
/// the floor) first, high window tops last; `salt` varies the pick among similar spots.
pub fn place_dummy(site: &ShinySite, salt: u64, practisable: &mut dyn FnMut(Vec2, (f32, f32)) -> bool) -> Result<(Spot, Vec2), DummyMiss> {
    let fp = site.footprint;
    let ledges = shiny::candidates(site);
    let mut miss = DummyMiss { ledges: ledges.len(), ..Default::default() };
    let mut mix = Mix(salt);
    let k = site.knight;
    // A comfortable distance: a little past the minimum.
    let want = (2.0 * site.min_distance).max(300.0);
    let mut spots: Vec<(f32, usize, f32)> = Vec::new();
    for (li, &(l, lo, hi)) in ledges.iter().enumerate() {
        let n = ((hi - lo) / DUMMY_STEP).ceil().clamp(1.0, 60.0) as usize;
        for i in 0..=n {
            let x = lo + (hi - lo) * i as f32 / n as f32;
            let p = Vec2::new(x, l.y);
            let free = p.distance(k) >= site.min_distance && site.occupancy.is_free(fp.rect(p), None) && !site.avoid.iter().any(|m| m.contains(x as i32, l.y as i32 - 1));
            if !free {
                continue;
            }
            let dy = (l.y - k.y).abs();
            let level = if dy < 4.0 {
                0.0
            } else if l.kind == LedgeKind::Ground {
                120.0
            } else {
                200.0 + 1.5 * dy
            };
            let cost = level + 0.5 * ((x - k.x).abs() - want).abs() + 160.0 * mix.f32();
            spots.push((cost, li, x));
        }
    }
    miss.spots = spots.len();
    spots.sort_by(|a, b| a.0.total_cmp(&b.0));
    // A few tries per ledge at most: an unreachable ledge shouldn't use up every plan.
    let mut per_ledge = vec![0u8; ledges.len()];
    for &(_, li, x) in &spots {
        if miss.tried >= DUMMY_PLANS || per_ledge[li] >= DUMMY_PER_LEDGE {
            continue;
        }
        per_ledge[li] += 1;
        let l = ledges[li].0;
        miss.tried += 1;
        if practisable(Vec2::new(x, l.y), (l.x0, l.x1))
            && let Some(placed) = shiny::spot_on(site.terrain, l, x)
        {
            return Ok(placed);
        }
    }
    Err(miss)
}

/// Stand-in drawing when the dummy's art isn't exported: a post with a crossbar and a
/// round stuffed head, `w`×`h` px, feet at the bottom centre. `swing` (−1..1) tilts it.
pub fn dummy_stand_in(w: u32, h: u32, swing: f32, alpha: f32) -> Image {
    let (w, h) = (w.max(8), h.max(16));
    let mut img = Image::new(w, h);
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
    if a == 0 {
        return img;
    }
    let (wf, hf) = (w as f32, h as f32);
    let tilt = swing.clamp(-1.0, 1.0) * 0.18;
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // Lean the whole thing around its foot.
            let up = hf - py;
            let cx = wf * 0.5 + tilt * up;
            let dx = px - cx;
            let pole = dx.abs() < (wf * 0.05).max(1.0) && py > hf * 0.2;
            let bar = (py - hf * 0.45).abs() < (hf * 0.025).max(1.0) && dx.abs() < wf * 0.42;
            let body = (dx / (wf * 0.24)).powi(2) + ((py - hf * 0.5) / (hf * 0.2)).powi(2) < 1.0;
            let head = dx.hypot(py - hf * 0.2) < wf * 0.2;
            let mask = head && dx.hypot(py - hf * 0.2) < wf * 0.13 && (py - hf * 0.2) > -wf * 0.1;
            let c = if mask {
                Some((236, 236, 230))
            } else if head || body {
                Some((112, 88, 120))
            } else if bar || pole {
                Some((70, 60, 54))
            } else {
                None
            };
            if let Some((r, g, b)) = c {
                img.px[(y * w + x) as usize] = premul(r, g, b, a);
            }
        }
    }
    img
}

// --- the lumafly ----------------------------------------------------------------------

/// How often a lumafly drifts by, how it flies, and how catchable it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LumaflyRules {
    /// Seconds between lumaflies, [min, max] (only counts while allowed).
    pub every_secs: [f32; 2],
    /// Seconds one stays before drifting off, [min, max].
    pub life_secs: [f32; 2],
    /// Cruising speed (px/s at scale 1).
    pub speed: f32,
    /// A swipe this close to it (px at scale 1, from his body centre) may catch it ...
    pub catch_reach: f32,
    /// ... with this chance; otherwise it darts away.
    pub catch_chance: f32,
}

impl Default for LumaflyRules {
    fn default() -> Self {
        Self { every_secs: [180.0, 480.0], life_secs: [70.0, 130.0], speed: 42.0, catch_reach: 70.0, catch_chance: 0.35 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlyState {
    /// Drifting to its next waypoint (and hovering a moment there).
    Drift,
    /// Swiped at and missed: darting away until then.
    Darting { until: f64 },
    /// Caught: a bright flare, then it slips out of his hands and floats up and away.
    Caught { at: f64 },
    /// Its time is up: floating up and away, fading.
    Leaving { since: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lumafly {
    pub id: u64,
    /// Centre of the glow (screen px), without the bob.
    pub pos: Vec2,
    pub vel: Vec2,
    pub target: Vec2,
    /// Hover at the waypoint until then.
    pub hover_until: f64,
    pub born: f64,
    pub leave_at: f64,
    pub state: FlyState,
    /// Bob / flicker phase (0..1).
    pub phase: f32,
    /// The monitor work area it keeps to.
    pub area: RectF,
}

/// A lumafly on screen this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShownFly {
    pub id: u64,
    /// Where it glows (bob included).
    pub pos: Vec2,
    pub alpha: f32,
    /// 1 = normal glow; up to ~2.5 in the catch flare.
    pub glow: f32,
    /// Leaving or caught: not worth chasing any more.
    pub leaving: bool,
    pub phase: f32,
}

/// What a swipe did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swipe {
    Caught,
    Missed,
    /// Nowhere near it (or it's gone / leaving).
    TooFar,
}

/// Seconds the leaving fade takes; the catch flare.
const LEAVE_SECS: f32 = 6.0;
const FLARE_SECS: f32 = 0.8;
/// Fade in (s).
const FLY_FADE_IN: f32 = 2.0;
/// Waypoints sit this far above a title bar (px at scale 1), [min, max].
const ABOVE_BAR: [f32; 2] = [50.0, 130.0];

/// The lumaflies (at most one) and when the next drifts by.
#[derive(Clone, Debug)]
pub struct Sky {
    flies: Vec<Lumafly>,
    next_id: u64,
    clock: f64,
    next_at: f64,
    mix: Mix,
}

/// Seed tag ("luma").
const FLY_TAG: u64 = 0x6c75_6d61;

impl Sky {
    pub fn new(seed: u64, rules: &LumaflyRules) -> Self {
        let mut s = Self { flies: Vec::new(), next_id: 1, clock: 0.0, next_at: 0.0, mix: Mix(seed ^ FLY_TAG) };
        s.schedule(rules);
        s
    }

    fn schedule(&mut self, rules: &LumaflyRules) {
        let [lo, hi] = rules.every_secs;
        self.next_at = self.clock + (lo + (hi - lo).max(0.0) * self.mix.f32()) as f64;
    }

    pub fn clock(&self) -> f64 {
        self.clock
    }

    pub fn flies(&self) -> &[Lumafly] {
        &self.flies
    }

    /// Advance the spawn timer: `true` when one should drift in now (then [`Self::spawn`]).
    pub fn due(&mut self, dt: f32, allowed: bool, rules: &LumaflyRules) -> bool {
        if !self.flies.is_empty() {
            return false;
        }
        if !allowed {
            self.next_at += dt as f64;
            return false;
        }
        if self.clock < self.next_at {
            return false;
        }
        self.schedule(rules);
        true
    }

    /// Waypoints: a little above the given title bars / ledges (`bars`: (x0, x1, y)), kept
    /// inside `area`; the middle of the area if there are none.
    fn waypoint(&mut self, area: RectF, bars: &[(f32, f32, f32)], scale: f32) -> Vec2 {
        let inside: Vec<&(f32, f32, f32)> = bars.iter().filter(|b| b.2 > area.top + 80.0 * scale && b.2 <= area.bottom + 1.0 && b.1 > area.left && b.0 < area.right).collect();
        let (x, y) = if inside.is_empty() {
            (area.left + area.width() * (0.2 + 0.6 * self.mix.f32()), area.top + area.height() * (0.3 + 0.4 * self.mix.f32()))
        } else {
            let b = inside[(self.mix.next() % inside.len() as u64) as usize];
            let x = b.0 + (b.1 - b.0) * (0.15 + 0.7 * self.mix.f32());
            let up = (ABOVE_BAR[0] + (ABOVE_BAR[1] - ABOVE_BAR[0]) * self.mix.f32()) * scale;
            (x, b.2 - up)
        };
        let m = 30.0 * scale;
        Vec2::new(x.clamp(area.left + m, area.right - m), y.clamp(area.top + m, area.bottom - m))
    }

    /// A lumafly drifts in from the side of `area` farther from `knight`, heading for a
    /// title bar. `bars`: (x0, x1, y) of ledges it may hover over.
    pub fn spawn(&mut self, area: RectF, bars: &[(f32, f32, f32)], knight: Vec2, rules: &LumaflyRules, scale: f32) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let from_left = knight.x > area.center().x;
        let x = if from_left { area.left + 10.0 * scale } else { area.right - 10.0 * scale };
        let y = area.top + area.height() * (0.25 + 0.35 * self.mix.f32());
        let target = self.waypoint(area, bars, scale);
        let [lo, hi] = rules.life_secs;
        let life = lo + (hi - lo).max(0.0) * self.mix.f32();
        let phase = self.mix.f32();
        self.flies.push(Lumafly {
            id,
            pos: Vec2::new(x, y),
            vel: Vec2::ZERO,
            target,
            hover_until: 0.0,
            born: self.clock,
            leave_at: self.clock + life as f64,
            state: FlyState::Drift,
            phase,
            area,
        });
        log::info!("a lumafly drifts in at ({x:.0},{y:.0}) (#{id})");
        id
    }

    /// Move them. `bars` as in [`Self::spawn`].
    pub fn step(&mut self, dt: f32, bars: &[(f32, f32, f32)], rules: &LumaflyRules, scale: f32) {
        self.clock += dt as f64;
        let now = self.clock;
        let mut flies = std::mem::take(&mut self.flies);
        for f in &mut flies {
            if matches!(f.state, FlyState::Drift | FlyState::Darting { .. }) && now >= f.leave_at {
                f.state = FlyState::Leaving { since: now };
            }
            if let FlyState::Darting { until } = f.state
                && now >= until
            {
                f.state = FlyState::Drift;
                f.target = self.waypoint(f.area, bars, scale);
            }
            let speed = rules.speed * scale;
            let (desired, agility) = match f.state {
                FlyState::Drift => {
                    let off = f.target - f.pos;
                    if off.length() < 6.0 * scale {
                        if f.hover_until <= 0.0 {
                            f.hover_until = now + (2.0 + 4.0 * self.mix.f32()) as f64;
                        } else if now >= f.hover_until {
                            f.hover_until = 0.0;
                            f.target = self.waypoint(f.area, bars, scale);
                        }
                        (Vec2::ZERO, 2.0)
                    } else {
                        // Slow down on arrival.
                        let s = speed * (off.length() / (40.0 * scale)).min(1.0);
                        (off.normalize_or_zero() * s, 1.2)
                    }
                }
                FlyState::Darting { .. } => {
                    let off = f.target - f.pos;
                    (off.normalize_or_zero() * speed * 3.5, 6.0)
                }
                FlyState::Caught { .. } | FlyState::Leaving { .. } => (Vec2::new(0.0, -speed * 0.9), 1.0),
            };
            f.vel += (desired - f.vel) * (agility * dt).min(1.0);
            f.pos += f.vel * dt;
            if matches!(f.state, FlyState::Drift | FlyState::Darting { .. }) {
                let m = 20.0 * scale;
                f.pos.x = f.pos.x.clamp(f.area.left + m, f.area.right - m);
                f.pos.y = f.pos.y.clamp(f.area.top + m, f.area.bottom - m);
            }
        }
        // Gone once faded out (or flown off the top).
        flies.retain(|f| match f.state {
            FlyState::Leaving { since } => ((now - since) as f32) < LEAVE_SECS,
            FlyState::Caught { at } => ((now - at) as f32) < FLARE_SECS + LEAVE_SECS,
            _ => true,
        });
        self.flies = flies;
    }

    /// The knight swipes (jumps, cups his hands) at fly `id` from `from` (his body centre).
    pub fn swipe(&mut self, id: u64, from: Vec2, rules: &LumaflyRules, scale: f32) -> Swipe {
        let now = self.clock;
        let roll = self.mix.f32();
        let away_x = self.mix.f32();
        let Some(f) = self.flies.iter_mut().find(|f| f.id == id) else { return Swipe::TooFar };
        if !matches!(f.state, FlyState::Drift | FlyState::Darting { .. }) || f.pos.distance(from) > rules.catch_reach * scale {
            return Swipe::TooFar;
        }
        if roll < rules.catch_chance {
            f.state = FlyState::Caught { at: now };
            f.vel = Vec2::ZERO;
            log::info!("the knight caught the lumafly (#{id}); it slips away");
            return Swipe::Caught;
        }
        // Darts off, away from him and up.
        let side = if f.pos.x >= from.x { 1.0 } else { -1.0 };
        let to = f.pos + Vec2::new(side * (120.0 + 120.0 * away_x) * scale, -(90.0 + 60.0 * away_x) * scale);
        let m = 30.0 * scale;
        f.target = Vec2::new(to.x.clamp(f.area.left + m, f.area.right - m), to.y.clamp(f.area.top + m, f.area.bottom - m));
        f.state = FlyState::Darting { until: now + 0.9 };
        Swipe::Missed
    }

    /// Send them off (a game came up, debug).
    pub fn dismiss(&mut self) {
        let now = self.clock;
        for f in &mut self.flies {
            if matches!(f.state, FlyState::Drift | FlyState::Darting { .. }) {
                f.state = FlyState::Leaving { since: now };
            }
        }
    }

    pub fn clear(&mut self) {
        self.flies.clear();
    }

    /// What to draw.
    pub fn shown(&self, scale: f32) -> Vec<ShownFly> {
        use std::f32::consts::TAU;
        let now = self.clock;
        self.flies
            .iter()
            .map(|f| {
                let bob = Vec2::new((TAU * ((now / 3.1).fract() as f32 + f.phase)).sin() * 3.0, (TAU * ((now / 1.7).fract() as f32 + f.phase)).sin() * 5.0) * scale;
                let fade_in = ((now - f.born) as f32 / FLY_FADE_IN).clamp(0.0, 1.0);
                let (alpha, glow, leaving) = match f.state {
                    FlyState::Drift | FlyState::Darting { .. } => (fade_in, 1.0, false),
                    FlyState::Caught { at } => {
                        let t = (now - at) as f32;
                        let flare = if t < FLARE_SECS { 1.0 + 1.5 * (1.0 - t / FLARE_SECS) } else { 1.0 };
                        (fade_in * (1.0 - ((t - FLARE_SECS).max(0.0) / LEAVE_SECS)).clamp(0.0, 1.0), flare, true)
                    }
                    FlyState::Leaving { since } => (fade_in * (1.0 - (now - since) as f32 / LEAVE_SECS).clamp(0.0, 1.0), 1.0, true),
                };
                ShownFly { id: f.id, pos: f.pos + bob, alpha, glow, leaving, phase: f.phase }
            })
            .collect()
    }
}

/// The lumafly's glow, `size` px square: a soft cyan halo, a bright pale core and two
/// faint flickering wings. `glow` > 1 brightens and widens it (the catch flare).
pub fn glow(size: u32, t: f64, phase: f32, glow: f32, alpha: f32) -> Image {
    use std::f32::consts::TAU;
    let size = size.max(7);
    let s = size as f32;
    let mut img = Image::new(size, size);
    let alpha = alpha.clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return img;
    }
    let flicker = 0.85 + 0.15 * (TAU * ((t / 0.9).fract() as f32 + phase)).sin();
    let g = glow.max(0.0);
    let (cx, cy) = (s * 0.5, s * 0.5);
    let halo_r = s * 0.22 * (0.8 + 0.2 * g);
    let core_r = (s * 0.05).max(0.7);
    let wing = (TAU * ((t * 9.0).fract() as f32 + phase)).sin().abs();
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let d = dx.hypot(dy);
            let halo = (-(d / halo_r).powi(2)).exp() * 0.55 * flicker * g.min(2.5);
            let core = (-(d / core_r).powi(2)).exp();
            // Wings: two small ellipses above-left and above-right of the body.
            let wing_at = |sx: f32| {
                let (u, v) = ((dx - sx * s * 0.07) / (s * 0.06), (dy + s * 0.05) / (s * 0.035 * (0.5 + 0.5 * wing)));
                if u * u + v * v < 1.0 { 0.35 } else { 0.0 }
            };
            let w = wing_at(-1.0) + wing_at(1.0);
            let a = (halo + core + w).min(1.0);
            let a = (a * alpha * 255.0) as u8;
            if a > 0 {
                // Cool cyan halo, near-white core.
                let warm = core.min(1.0);
                let r = (150.0 + 105.0 * warm) as u8;
                let gr = (235.0 + 20.0 * warm) as u8;
                img.px[(y * size + x) as usize] = premul(r, gr, 255, a);
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::{Monitor, WindowInfo};
    use crate::geom::RectI;
    use crate::terrain::TerrainOptions;

    fn desk(windows: Vec<WindowInfo>) -> (DesktopSnapshot, Terrain) {
        let d = DesktopSnapshot {
            generation: 1,
            monitors: vec![Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 }],
            windows,
        };
        let t = Terrain::from_desktop(&d, &TerrainOptions::default());
        (d, t)
    }

    #[test]
    fn a_dummy_turns_up_on_schedule_wears_out_and_breaks() {
        let rules = DummyRules { every_secs: [10.0, 10.0], life_secs: [60.0, 60.0], min_distance: 0.0, ..Default::default() };
        let mut yard = Yard::new(1, &rules);
        let (d, t) = desk(vec![]);
        let mut due = false;
        for _ in 0..(9.0 / 0.1) as usize {
            due |= yard.due(0.1, true, &rules);
        }
        assert!(!due, "not before its time");
        // Not while held back: the timer waits.
        for _ in 0..100 {
            assert!(!yard.due(0.1, false, &rules));
        }
        assert!((0..30).any(|_| yard.due(0.1, true, &rules)), "due now");
        yard.add(Spot::Ground { pos: Vec2::new(500.0, 1040.0) }, &rules);
        let s = yard.shown(&d, &t, 40.0, &|_| true).expect("shown");
        assert_eq!(s.pos, Vec2::new(500.0, 1040.0));
        assert!(s.alpha < 0.2, "fading in");
        // A blow lands after its delay, once.
        yard.strike(DummyHit { heavy: true, from_right: false }, 0.3);
        yard.due(0.2, true, &rules);
        assert!(yard.take_landed().is_empty());
        yard.due(0.2, true, &rules);
        assert_eq!(yard.take_landed(), vec![DummyHit { heavy: true, from_right: false }]);
        assert!(yard.take_landed().is_empty());
        // Standing wears it out: it breaks at the end of its life (time's doing).
        let mut broke_at = None;
        for i in 0..(70.0 / 0.1) as usize {
            assert!(!yard.due(0.1, true, &rules), "one at a time");
            let b = yard.take_broken();
            if !b.is_empty() {
                assert_eq!(b, vec![DummyBreak { id: 1, by: Breaker::Wear }]);
                broke_at = Some(i as f32 * 0.1);
                break;
            }
            assert!(yard.shown(&d, &t, 40.0, &|_| true).is_some_and(|s| !s.broken));
        }
        // 60 s of life, less the heavy blow's 3 % (1.8 s), from 0.4 s in.
        let at = broke_at.expect("broke");
        assert!((57.0..59.0).contains(&at), "{at}");
        // A quick fade (unseen by the brain), then gone.
        let s = yard.shown(&d, &t, 40.0, &|_| true).expect("fading");
        assert!(s.broken && s.alpha > 0.5);
        yard.strike(DummyHit { heavy: false, from_right: false }, 0.0);
        yard.due(0.2, true, &rules);
        assert!(yard.take_landed().is_empty(), "no blows land on a broken dummy");
        assert!(yard.shown(&d, &t, 40.0, &|_| true).is_some_and(|s| s.alpha < 0.5));
        yard.due(0.2, true, &rules);
        assert!(yard.shown(&d, &t, 40.0, &|_| true).is_none());
        assert!(yard.dummy().is_none());
        // The next one on the usual schedule, counted from when it went.
        assert!(!(0..95).any(|_| yard.due(0.1, true, &rules)), "not before its time");
        assert!((0..10).any(|_| yard.due(0.1, true, &rules)), "due again ~10 s after it went");
    }

    #[test]
    fn his_blows_wear_it_and_a_session_never_breaks_a_fresh_one() {
        let rules = DummyRules::default();
        let mut yard = Yard::new(4, &rules);
        let id = yard.add(Spot::Ground { pos: Vec2::new(500.0, 1040.0) }, &rules);
        // The heaviest session there is: six drills alternating cyclones (two light, one
        // heavy) and three-slash combos, over a minute.
        for drill in 0..6 {
            let blows: &[bool] = if drill % 2 == 0 { &[false, false, true] } else { &[false, false, false] };
            for &heavy in blows {
                yard.strike(DummyHit { heavy, from_right: false }, 0.1);
                yard.due(0.5, true, &rules);
            }
            yard.due(8.0, true, &rules);
        }
        assert!(yard.take_broken().is_empty());
        let w = yard.wear().unwrap();
        assert!((0.2..0.4).contains(&w), "a good dent, not the end: {w}");
        // A few more sessions finish it, under his nail.
        let mut sessions = 1;
        while yard.take_broken().is_empty() {
            sessions += 1;
            assert!(sessions < 8, "{:?}", yard.wear());
            for _ in 0..15 {
                yard.strike(DummyHit { heavy: false, from_right: false }, 0.0);
                yard.due(0.3, true, &rules);
            }
            let b = yard.take_broken();
            if !b.is_empty() {
                assert_eq!(b, vec![DummyBreak { id, by: Breaker::Knight }]);
                break;
            }
        }
        assert!(sessions >= 3, "a good few sessions: {sessions}");
        // Over its life alone it would have lasted 13-18 min.
        let mut y = Yard::new(6, &rules);
        y.add(Spot::Ground { pos: Vec2::ZERO }, &rules);
        let mut t = 0.0;
        while y.take_broken().is_empty() {
            y.due(1.0, true, &rules);
            t += 1.0;
            assert!(t < 1200.0);
        }
        assert!((780.0..=1081.0).contains(&t), "{t}");
    }

    #[test]
    fn five_clicks_break_it_and_the_next_one_waits_longer() {
        let rules = DummyRules { every_secs: [100.0, 100.0], ..Default::default() };
        let (d, t) = desk(vec![]);
        let mut yard = Yard::new(7, &rules);
        let id = yard.add(Spot::Ground { pos: Vec2::new(500.0, 1040.0) }, &rules);
        assert_eq!(yard.click(id + 1, &rules), None, "another dummy");
        for n in 1..=4 {
            assert_eq!(yard.click(id, &rules), Some(false), "click {n}");
        }
        assert!(yard.take_landed().is_empty(), "not his blows (the app swings it itself)");
        assert_eq!(yard.click(id, &rules), Some(true), "the fifth breaks it");
        assert_eq!(yard.take_broken(), vec![DummyBreak { id, by: Breaker::User }]);
        assert_eq!(yard.click(id, &rules), None, "broken: no more");
        assert!(!yard.moved(id, Spot::Ground { pos: Vec2::ZERO }), "nor dragged");
        yard.due(0.5, true, &rules);
        assert!(yard.shown(&d, &t, 40.0, &|_| true).is_none());
        // 2.5 × the usual 100 s before the next.
        let mut secs = 0.0;
        while !yard.due(1.0, true, &rules) {
            secs += 1.0;
            assert!(secs < 400.0);
        }
        assert!((245.0..=252.0).contains(&secs), "{secs}");
        // A worn dummy takes fewer clicks.
        let id = yard.add(Spot::Ground { pos: Vec2::new(500.0, 1040.0) }, &rules);
        for _ in 0..25 {
            yard.strike(DummyHit { heavy: true, from_right: false }, 0.0);
            yard.due(0.1, true, &rules);
        }
        assert!(yard.take_broken().is_empty());
        let clicks = (1..=5).find(|_| yard.click(id, &rules) == Some(true)).expect("broke");
        assert!(clicks <= 2, "{clicks}");
    }

    #[test]
    fn a_placed_or_dragged_dummy_wears_and_breaks_too() {
        let rules = DummyRules { life_secs: [100.0, 100.0], ..Default::default() };
        let mut yard = Yard::new(8, &rules);
        let id = yard.add(Spot::Ground { pos: Vec2::new(500.0, 1040.0) }, &rules);
        yard.pin(id);
        yard.due(50.0, true, &rules);
        assert!(yard.moved(id, Spot::Ground { pos: Vec2::new(900.0, 1040.0) }));
        assert!((yard.wear().unwrap() - 0.5).abs() < 0.01, "wear goes with it");
        yard.due(51.0, true, &rules);
        assert_eq!(yard.take_broken(), vec![DummyBreak { id, by: Breaker::Wear }]);
    }

    #[test]
    fn a_dummy_on_a_window_goes_with_it() {
        let rules = DummyRules::default();
        let mut w = WindowInfo { id: 7, rect: RectI::new(300, 400, 1200, 900), title: String::new(), class: String::new(), process: "x.exe".into(), pid: 1, maximized: false, fullscreen: false, foreground: false, topmost: false };
        let (d, t) = desk(vec![w.clone()]);
        let mut yard = Yard::new(2, &rules);
        yard.add(Spot::Window { id: 7, anchor: crate::discovery::Anchor { fx: 0.5, dy: 0.0 } }, &rules);
        let s = yard.shown(&d, &t, 40.0, &|_| true).expect("on the title bar");
        assert_eq!(s.pos.y, 400.0);
        // The window goes fullscreen: the dummy is gone.
        w.fullscreen = true;
        let (d, t) = desk(vec![w]);
        assert!(yard.shown(&d, &t, 40.0, &|_| true).is_none());
        assert!(yard.dummy().is_none());
    }

    #[test]
    fn a_dragged_dummy_stays_where_the_user_dropped_it() {
        let rules = DummyRules::default();
        let w = WindowInfo { id: 7, rect: RectI::new(300, 400, 1200, 900), title: String::new(), class: String::new(), process: "x.exe".into(), pid: 1, maximized: false, fullscreen: false, foreground: false, topmost: false };
        let (d, t) = desk(vec![w]);
        let mut yard = Yard::new(3, &rules);
        let id = yard.add(Spot::Ground { pos: Vec2::new(500.0, 1040.0) }, &rules);
        assert!(yard.relocatable(id), "one the schedule put up may be moved when he can't reach it");
        let leave_at = yard.dummy().unwrap().leave_at;
        yard.strike(DummyHit { heavy: false, from_right: true }, 0.5);
        // Dragged onto the window's title bar: same dummy and stay, the new spot, pinned.
        assert!(yard.moved(id, Spot::Window { id: 7, anchor: crate::discovery::Anchor { fx: 0.5, dy: 0.0 } }));
        let s = yard.shown(&d, &t, 40.0, &|_| true).expect("on the title bar");
        assert_eq!((s.id, s.pos), (id, Vec2::new(750.0, 400.0)));
        assert_eq!(yard.dummy().unwrap().leave_at, leave_at);
        assert!(!yard.relocatable(id), "never moved away from where the user put it");
        yard.due(1.0, true, &rules);
        assert!(yard.take_landed().is_empty(), "the blow aimed at the old spot misses");
        // The Test tab's placement pins it too; another id is not this dummy.
        let id2 = yard.add(Spot::Ground { pos: Vec2::new(500.0, 1040.0) }, &rules);
        assert!(!yard.moved(id, Spot::Ground { pos: Vec2::ZERO }));
        yard.pin(id2);
        assert!(!yard.relocatable(id2));
    }

    #[test]
    fn the_dummy_stands_only_on_settled_ledges_and_never_on_a_game_monitor() {
        use crate::discovery::{Footprint, LASTING_SECS};
        use crate::lasting::LedgeAges;
        use crate::occupancy::Occupancy;
        use crate::shiny::{ShinySite, place};
        // Two monitors; the right one runs a game. A window on each.
        let d = DesktopSnapshot {
            generation: 1,
            monitors: vec![
                Monitor { rect: RectI::new(0, 0, 1920, 1080), work: RectI::new(0, 0, 1920, 1040), primary: true, scale_milli: 1000 },
                Monitor { rect: RectI::new(1920, 0, 3840, 1080), work: RectI::new(1920, 0, 3840, 1040), primary: false, scale_milli: 1000 },
            ],
            windows: vec![
                WindowInfo { id: 1, rect: RectI::new(300, 400, 1200, 900), title: String::new(), class: String::new(), process: "a.exe".into(), pid: 1, maximized: false, fullscreen: false, foreground: false, topmost: false },
                WindowInfo { id: 2, rect: RectI::new(2200, 400, 3200, 900), title: String::new(), class: String::new(), process: "b.exe".into(), pid: 2, maximized: false, fullscreen: false, foreground: false, topmost: false },
            ],
        };
        let t = Terrain::from_desktop(&d, &TerrainOptions::default());
        let aged = |secs: f32| {
            let mut a = LedgeAges::default();
            let mut now = 0.0;
            while now <= secs as f64 {
                a.observe(&t, now);
                now += 1.0;
            }
            a
        };
        let occ = Occupancy::default();
        let game = [RectI::new(1920, 0, 3840, 1080)];
        let fp = Footprint { w: 30.0, h: 160.0, room: 140.0 };
        for (age, window_ok) in [(10.0, false), (60.0, true)] {
            let ages = aged(age);
            let mut on_window = 0;
            for salt in 0..60 {
                let site = ShinySite { terrain: &t, ages: &ages, now: age as f64, min_age: LASTING_SECS, occupancy: &occ, footprint: fp, knight: Vec2::new(100.0, 1040.0), min_distance: 220.0, area: None, avoid: &game, reachable: &|_| true };
                let (spot, pos) = place(&site, &[], salt).expect("somewhere");
                assert!(pos.x < 1920.0, "never on the game's monitor: {pos:?}");
                if let Spot::Window { .. } = spot {
                    on_window += 1;
                }
            }
            assert_eq!(on_window > 0, window_ok, "window ledges {age} s old: {on_window}");
        }
    }

    #[test]
    fn hit_clips_swing_away_from_the_blow() {
        assert_eq!(DummyHit { heavy: false, from_right: true }.clip(), "Light Hit L");
        assert_eq!(DummyHit { heavy: true, from_right: false }.clip(), "Heavy Hit R");
        // The user's clicks: from the side clicked on.
        let foot = Vec2::new(500.0, 1040.0);
        assert_eq!(DummyHit::clicked(Vec2::new(510.0, 950.0), foot).clip(), "Light Hit L");
        assert_eq!(DummyHit::clicked(Vec2::new(490.0, 950.0), foot).clip(), "Light Hit R");
    }

    fn sky() -> (Sky, LumaflyRules, RectF) {
        let rules = LumaflyRules::default();
        (Sky::new(3, &rules), rules, RectF::new(0.0, 0.0, 1920.0, 1040.0))
    }

    #[test]
    fn a_lumafly_drifts_over_the_title_bars_and_leaves_in_time() {
        let (mut sky, rules, area) = sky();
        let bars = [(200.0, 900.0, 400.0), (1000.0, 1700.0, 250.0)];
        sky.spawn(area, &bars, Vec2::new(1500.0, 1040.0), &rules, 1.0);
        assert!(sky.flies()[0].pos.x < 100.0, "comes in from the side away from him");
        let mut hovered = 0;
        for _ in 0..(60.0 / 0.05) as usize {
            sky.step(0.05, &bars, &rules, 1.0);
            let f = sky.flies()[0];
            assert!(area.contains(f.pos), "keeps to its monitor: {:?}", f.pos);
            let over_a_bar = bars.iter().any(|b| f.pos.x >= b.0 - 5.0 && f.pos.x <= b.1 + 5.0 && f.pos.y < b.2 - 40.0 && f.pos.y > b.2 - 140.0);
            if f.vel.length() < 5.0 && over_a_bar {
                hovered += 1;
            }
        }
        assert!(hovered > 0, "hovers above title bars");
        for _ in 0..(160.0 / 0.05) as usize {
            sky.step(0.05, &bars, &rules, 1.0);
        }
        assert!(sky.flies().is_empty(), "drifted off after its life");
    }

    #[test]
    fn a_swipe_close_by_catches_it_sometimes_otherwise_it_darts_off() {
        let (mut sky, rules, area) = sky();
        let mut caught = 0;
        let mut missed = 0;
        for i in 0..40 {
            sky.clear();
            sky.spawn(area, &[], Vec2::ZERO, &rules, 1.0);
            let f = sky.flies()[0];
            assert_eq!(sky.swipe(f.id, f.pos + Vec2::new(400.0, 0.0), &rules, 1.0), Swipe::TooFar, "out of reach");
            match sky.swipe(f.id, f.pos + Vec2::new(20.0, 10.0 + i as f32), &rules, 1.0) {
                Swipe::Caught => {
                    caught += 1;
                    assert!(sky.shown(1.0)[0].leaving, "caught: no longer chased");
                    assert!(sky.shown(1.0)[0].glow > 2.0, "the flare");
                }
                Swipe::Missed => {
                    missed += 1;
                    let before = sky.flies()[0].pos;
                    for _ in 0..10 {
                        sky.step(0.05, &[], &rules, 1.0);
                    }
                    assert!(sky.flies()[0].pos.distance(before) > 40.0, "darts away");
                }
                Swipe::TooFar => panic!("within reach"),
            }
        }
        assert!(caught >= 5 && missed >= 15, "caught {caught}, missed {missed}");
    }

    #[test]
    fn the_glow_is_soft_and_brighter_in_the_flare() {
        let calm = glow(24, 0.0, 0.0, 1.0, 1.0);
        let flare = glow(24, 0.0, 0.0, 2.5, 1.0);
        let sum = |img: &Image| img.px.iter().map(|p| (p >> 24) as u64).sum::<u64>();
        assert!(sum(&flare) > sum(&calm));
        let c = calm.px[(12 * 24 + 12) as usize] >> 24;
        assert!(c > 200, "bright core: {c}");
        assert_eq!(calm.px[0] >> 24, 0, "transparent corners");
        assert!(dummy_stand_in(40, 120, 0.0, 1.0).px.iter().any(|p| p >> 24 > 0));
    }
}
