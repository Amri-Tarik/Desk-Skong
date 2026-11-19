//! Graph construction: candidate traversals between surface pairs, pruned analytically and
//! validated by forward-simulating a cloned [`Character`] against (a local copy of) the
//! real terrain. Results are cached per pair, keyed by the geometry they depend on, so a
//! rebuild after one window moved only re-simulates the pairs near that window.
//!
//! Everything a pair's result depends on is read from the *local* terrain (the part of the
//! terrain around the pair that a run can touch), which is what the cache key hashes.
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

use super::MoveKind;
use super::script::{Climb, ClimbWall, Exec, Outcome, Recipe, SHORT_RUNUP, Script, Stage, Start};
use super::surface::{Surface, SurfaceId, all_walls};
use crate::controller::{Abilities, Character, CharacterParams, Gait, MoveEvent, MoveInput, MoveState, Side};
use crate::geom::{RectF, Vec2};
use crate::terrain::{Facing, LedgeKind, Terrain, Wall, WallKind};

/// Fixed step used to validate edges (the app ticks at 120 Hz).
pub(crate) const SIM_DT: f32 = 1.0 / 120.0;
/// Walls considered per climbing pair.
const MAX_CLIMB_WALLS: usize = 3;
/// Perturbed re-runs a climb must also pass.
const PERTURBATIONS: u8 = 1;
/// A climb run without a new high point for this long (s) is abandoned.
const CLIMB_STALL: f32 = 1.1;
/// Slowest take-off speed (fraction of the gait speed) a perturbed run uses; just above what
/// the follower's approach accepts.
const RUN_FRACTION_MIN: f32 = 0.82;

/// A validated traversal from one surface to another.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub from: SurfaceId,
    pub to: SurfaceId,
    pub kind: MoveKind,
    /// Gait the script runs at (approach to the take-off may use the style's gait).
    pub gait: Gait,
    /// Take-off feet x on `from`.
    pub takeoff: f32,
    /// Where the validation run came to rest on `to` (feet).
    pub land: Vec2,
    /// Seconds from take-off to standing on `to` (validation run).
    pub duration: f32,
    /// Walls used, for climbs ([`MoveKind::WallJump`] edges, and climbs that needed no wall
    /// jump in the end).
    pub climb: Option<Climb>,
    pub(crate) script: Script,
}

/// Timings and counters of the last build.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildStats {
    pub surfaces: usize,
    pub edges: usize,
    /// Ordered surface pairs that survived analytic pruning.
    pub pairs: usize,
    /// Pair results reused from the previous graph.
    pub cache_hits: usize,
    /// Forward simulations run (one per candidate script).
    pub sims: usize,
    /// Simulated ticks in total.
    pub ticks: usize,
    /// Climbs that worked once but failed from a perturbed start (rejected).
    pub brittle: usize,
    /// Wall-climb simulations (perturbed re-runs included) and their ticks.
    pub climb_sims: usize,
    pub climb_ticks: usize,
    pub millis: f32,
}

/// Which styles an edge search serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tier {
    /// Walk gait; walk / drop / single jump.
    Stroll,
    /// Run gait; everything but superdash.
    Normal,
    /// Superdash only (tried when nothing else connects the pair).
    Hurry,
}

/// Why a candidate failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fail {
    /// Came to rest on another surface; `above`: higher than the target, `near`: at the
    /// target x (something sits on top of the target there).
    Landed { above: bool, near: bool, source: bool },
    /// Can no longer reach the target (fell below it, or too far to steer to before then).
    Short,
    /// Could not start, left the take-off surface, stuck, out of the region or out of time.
    Other,
}

/// Pair result without the surface ids (what the cache stores).
#[derive(Clone, Copy, Debug)]
struct Found {
    kind: MoveKind,
    duration: f32,
    land: Vec2,
    script: Script,
}

/// Results for one ordered pair, per tier (stroll, normal, hurry).
type PairResult = [Option<Found>; 3];

/// Pair results of one build, keyed by a hash of everything they depend on.
#[derive(Clone, Default)]
pub(crate) struct EdgeCache {
    fingerprint: u64,
    map: Arc<HashMap<u64, PairResult>>,
}

impl std::fmt::Debug for EdgeCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EdgeCache({} pairs)", self.map.len())
    }
}

/// Analytic reach of the kit (px, s), used only to prune hopeless candidates.
#[derive(Clone, Copy, Debug)]
struct Reach {
    half: f32,
    height: f32,
    step: f32,
    gravity: f32,
    max_fall: f32,
    jump: f32,
    double: f32,
    apex_time: f32,
    dash: f32,
}

impl Reach {
    fn new(p: &CharacterParams) -> Self {
        let hit = p.hitbox_px();
        Self {
            half: hit.x * 0.5,
            height: hit.y,
            step: p.step_height_px(),
            gravity: p.gravity_px(),
            max_fall: p.max_fall_speed_px(),
            jump: p.px(p.jump_height()),
            double: p.px(p.double_jump_height()),
            apex_time: p.jump_time + p.jump_speed / p.gravity,
            dash: p.px(p.dash_distance()),
        }
    }

    /// Free-fall time from rest through `drop` px (0 if not above).
    fn fall_time(&self, drop: f32) -> f32 {
        (2.0 * drop.max(0.0) / self.gravity).sqrt()
    }

    /// Time (upper bound, honouring terminal speed) to fall `dy` px with initial speed `vy`.
    fn time_to_fall(&self, vy: f32, dy: f32) -> f32 {
        let (g, vmax) = (self.gravity, self.max_fall);
        let vy = vy.min(vmax);
        let t_cap = (vmax - vy) / g;
        let d_cap = 0.5 * (vy + vmax) * t_cap;
        if dy <= d_cap { (-vy + (vy * vy + 2.0 * g * dy).sqrt()) / g } else { t_cap + (dy - d_cap) / vmax }
    }

    /// Max horizontal travel landing `rise` px above take-off at speed `v`: walking off,
    /// single jump, jump + double jump.
    fn walk_off(&self, v: f32, rise: f32) -> f32 {
        v * self.fall_time(-rise)
    }
    fn single(&self, v: f32, rise: f32) -> f32 {
        v * (self.apex_time + self.fall_time(self.jump - rise))
    }
    fn double(&self, v: f32, rise: f32) -> f32 {
        v * (2.0 * self.apex_time + self.fall_time(self.jump + self.double - rise))
    }
}

/// Generous slack on analytic reach: pruning must never drop a feasible pair.
fn within(gap: f32, reach: f32) -> bool {
    gap <= reach * 1.15 + 24.0
}

struct Builder<'a> {
    terrain: &'a Terrain,
    params: CharacterParams,
    abilities: Abilities,
    surfaces: &'a [Surface],
    reach: Reach,
    /// Terrain around the current pair, and its walls plus the bounds sides.
    local: Terrain,
    local_walls: Vec<Wall>,
    /// Terrain for the current simulation (a subset of `local`).
    sim: Terrain,
    events: Vec<MoveEvent>,
    candidates: Vec<Script>,
    old: &'a HashMap<u64, PairResult>,
    new: HashMap<u64, PairResult>,
    fingerprint: u64,
    stats: BuildStats,
}

/// Build the edges between `surfaces`. `prev` is the previous build's cache (may be empty).
pub(crate) fn build_edges(
    terrain: &Terrain,
    template: &Character,
    surfaces: &[Surface],
    prev: &EdgeCache,
) -> (Vec<Edge>, EdgeCache, BuildStats) {
    let started = Instant::now();
    let mut h = DefaultHasher::new();
    format!("{:?}|{:?}|{}", template.params, template.abilities, SIM_DT).hash(&mut h);
    let fingerprint = h.finish();
    let empty = HashMap::new();
    let old = if prev.fingerprint == fingerprint { prev.map.as_ref() } else { &empty };
    let mut b = Builder {
        terrain,
        params: template.params,
        abilities: template.abilities,
        surfaces,
        reach: Reach::new(&template.params),
        local: Terrain::default(),
        local_walls: Vec::new(),
        sim: Terrain::default(),
        events: Vec::new(),
        candidates: Vec::new(),
        old,
        new: HashMap::new(),
        fingerprint,
        stats: BuildStats { surfaces: surfaces.len(), ..BuildStats::default() },
    };
    let mut edges = Vec::new();
    for from in 0..surfaces.len() {
        for to in 0..surfaces.len() {
            if from != to {
                b.connect(from, to, &mut edges);
            }
        }
    }
    b.stats.edges = edges.len();
    b.stats.millis = started.elapsed().as_secs_f32() * 1000.0;
    let cache = EdgeCache { fingerprint, map: Arc::new(b.new) };
    (edges, cache, b.stats)
}

impl Builder<'_> {
    fn connect(&mut self, from: SurfaceId, to: SurfaceId, edges: &mut Vec<Edge>) {
        let (s, t) = (self.surfaces[from], self.surfaces[to]);
        let tiers = self.possible_tiers(&s, &t);
        if tiers.iter().all(|t| !t) {
            return;
        }
        let region = self.region(&s, &t);
        let local_hash = self.fill_local(region);
        if self.drop_is_blocked(&s, &t) {
            return;
        }
        self.stats.pairs += 1;
        let key = self.key(&s, &t, local_hash);
        let found = match self.old.get(&key) {
            Some(found) => {
                self.stats.cache_hits += 1;
                *found
            }
            None => self.search_pair(&s, &t, region, tiers),
        };
        self.new.insert(key, found);
        for f in found.into_iter().flatten() {
            edges.push(Edge {
                from,
                to,
                kind: f.kind,
                gait: f.script.gait,
                takeoff: f.script.takeoff,
                land: f.land,
                duration: f.duration,
                climb: f.script.climb,
                script: f.script,
            });
        }
    }

    /// Per tier: could any recipe possibly connect `s` to `t`? (Purely analytic.)
    fn possible_tiers(&self, s: &Surface, t: &Surface) -> [bool; 3] {
        let r = self.reach;
        let p = &self.params;
        let ab = self.abilities;
        let rise = s.y - t.y;
        let gap = (gap_between(s, t) - 2.0 * r.half).max(0.0);
        let walk = p.gait_speed_px(Gait::Walk);
        let run = p.gait_speed_px(Gait::Run);
        let low_enough = rise <= r.step;
        // Strolls have the whole kit but dashes: walk-offs, jumps, double jumps, wall climbs.
        let stroll = (low_enough && within(gap, r.walk_off(walk, rise)))
            || (rise < r.jump - 4.0 && within(gap, r.single(walk, rise)))
            || (ab.double_jump && rise < r.jump + r.double - 4.0 && within(gap, r.double(walk, rise)))
            || (ab.wall_jump && rise > 0.0);
        let dash = if ab.dash { r.dash } else { 0.0 };
        let normal_jump = if ab.double_jump && rise < r.jump + r.double - 4.0 {
            within(gap, r.double(run, rise) + dash)
        } else {
            rise < r.jump - 4.0 && within(gap, r.single(run, rise) + dash)
        };
        let normal = (low_enough && within(gap, r.walk_off(run, rise) + dash)) || normal_jump || (ab.wall_jump && rise > 0.0);
        // Superdash is the long-range fallback, for gaps a running jump cannot span.
        let far = !within(gap, r.single(run, rise.min(0.0)));
        let hurry = ab.superdash && low_enough && far && separated(s, t).is_some();
        [stroll, normal, hurry]
    }

    /// `t` lies below `s` and a ledge spans the whole fall between them.
    fn drop_is_blocked(&self, s: &Surface, t: &Surface) -> bool {
        let h = self.reach.half;
        if t.y <= s.y + self.reach.step {
            return false;
        }
        let (a, b) = match separated(s, t) {
            Some(Side::Right) => (s.hi - h, t.lo + h),
            Some(Side::Left) => (t.hi - h, s.lo + h),
            None => (s.lo.min(t.lo) - h, s.hi.max(t.hi) + h),
        };
        self.local.ledges.iter().any(|u| u.y > s.y + 1.0 && u.y < t.y - 1.0 && u.x0 <= a && u.x1 >= b)
    }

    /// Simulation region (feet positions) for a pair: leaving it fails the run.
    fn region(&self, s: &Surface, t: &Surface) -> RectF {
        let r = self.reach;
        // Horizontal slack for overshoot / wall kicks; vertical room up to a double jump above
        // the take-off (or a single jump above a higher target, for climbs).
        let mx = r.dash + 2.0 * r.half + 32.0;
        let top = (s.y - r.jump - r.double).min(t.y - r.jump) - 16.0;
        // Side by side: every candidate leaves from the facing end of `s` (after a run-up)
        // and aims at the facing part of `t`, so only the stretch between them matters. This
        // keeps the per-tick terrain small when `s` or `t` is a monitor-wide floor.
        let reach_in = self.runup(Gait::Sprint) + 2.0 * r.half + mx;
        let (left, right) = match separated(s, t) {
            Some(Side::Right) => ((s.hi - reach_in).max(s.x0), (t.lo + reach_in).min(t.x1)),
            Some(Side::Left) => ((t.hi - reach_in).max(t.x0), (s.lo + reach_in).min(s.x1)),
            None => (s.x0.min(t.x0), s.x1.max(t.x1)),
        };
        RectF::new(left - mx, top, right + mx, s.y.max(t.y) + 1.0)
    }

    /// Copy the terrain overlapping `region` (grown by the body) into `self.local`; returns
    /// a hash of that geometry.
    fn fill_local(&mut self, region: RectF) -> u64 {
        copy_around(self.terrain, &mut self.local, region, self.reach);
        let l = &self.local;
        self.local_walls.clear();
        self.local_walls.extend(all_walls(l));
        let mut h = DefaultHasher::new();
        let bits = |h: &mut DefaultHasher, v: &[f32]| v.iter().for_each(|f| f.to_bits().hash(h));
        bits(&mut h, &[l.bounds.left, l.bounds.top, l.bounds.right, l.bounds.bottom]);
        for w in &l.windows {
            w.id.hash(&mut h);
            bits(&mut h, &[w.rect.left, w.rect.top, w.rect.right, w.rect.bottom]);
        }
        for e in &l.ledges {
            (e.kind as u8, e.owner).hash(&mut h);
            bits(&mut h, &[e.x0, e.x1, e.y]);
        }
        for w in &l.walls {
            (w.face == Facing::Left, w.owner).hash(&mut h);
            bits(&mut h, &[w.x, w.y0, w.y1]);
        }
        h.finish()
    }

    fn key(&self, s: &Surface, t: &Surface, local: u64) -> u64 {
        let mut h = DefaultHasher::new();
        (self.fingerprint, local).hash(&mut h);
        for v in [s.x0, s.x1, s.y, s.lo, s.hi, t.x0, t.x1, t.y, t.lo, t.hi] {
            v.to_bits().hash(&mut h);
        }
        h.finish()
    }

    /// Search every possible tier of a pair. Tiers share what they learn: a landing x where a
    /// run was caught by a surface sitting right above the target is not retried (more
    /// power or speed will not help), and the stroll's winner is tried first at run speed.
    fn search_pair(&mut self, s: &Surface, t: &Surface, region: RectF, tiers: [bool; 3]) -> PairResult {
        let mut out = [None; 3];
        let mut blocked: Vec<f32> = Vec::new();
        if tiers[1] {
            out[1] = self.search(Tier::Normal, s, t, region, None, &mut blocked);
        }
        // Strolling uses everything but dashes, at walking pace: if running could not connect
        // the pair at all, walking will not either. (A run that dashed may still be walkable
        // with a climb instead, so any run result is worth a stroll search.)
        if tiers[0] && out[1].is_some() {
            let hint = out[1].map(|f| f.script.recipe);
            out[0] = self.search(Tier::Stroll, s, t, region, hint, &mut blocked);
        }
        if tiers[2] && out[1].is_none() {
            out[2] = self.search(Tier::Hurry, s, t, region, None, &mut blocked);
        }
        out
    }

    /// Try the tier's candidate scripts in order of preference; first success wins. A climb
    /// only counts if it also succeeds from a perturbed start. (No second, alternative climb
    /// per pair: it cost ~10% of build time, and the follower's slip memory already falls
    /// back on other routes.)
    fn search(
        &mut self,
        tier: Tier,
        s: &Surface,
        t: &Surface,
        region: RectF,
        hint: Option<Recipe>,
        blocked: &mut Vec<f32>,
    ) -> Option<Found> {
        let mut cands = std::mem::take(&mut self.candidates);
        cands.clear();
        self.candidates_for(tier, s, t, &mut cands);
        if let Some(r) = hint
            && let Some(i) = cands.iter().position(|c| c.recipe == r)
        {
            // Retry the other tier.s winner first (likely to work at this pace too), but never
            // ahead of a drop-through, which beats walking to an edge.
            let c = cands.remove(i);
            let at = usize::from(cands.first().is_some_and(|c| c.recipe == Recipe::DropThrough));
            cands.insert(at, c);
        }
        let mut found: Option<Found> = None;
        let mut caught = false;
        let mut dead_walls: Vec<f32> = Vec::new();
        let (mut stalled_walls, mut short_walls): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
        // Jumps tried, and whether any of them failed in a way a climb could fix (bounced back
        // onto the source off a wall, or fell short). Jumps that were all caught by other
        // surfaces on the way say the same of climbs.
        let (mut jumps_tried, mut climb_could_help) = (false, false);
        for c in &cands {
            if c.recipe == Recipe::Climb && jumps_tried && !climb_could_help {
                continue;
            }
            // Caught on the way by another surface: an air dash (flying further at the same
            // heights) will not do better, unless it is meant to shadow-dash through a wall.
            let dash_jump = matches!(c.recipe, Recipe::Jump { dash: true, .. });
            let dead_wall = c.climb.is_some_and(|cl| dead_walls.contains(&cl.wall.x));
            // Climb variants only where the plain climb up that wall showed they could help: a
            // shaft where it stalled, a dash finish where it topped out but fell short.
            let needless = c.climb.is_some_and(|cl| {
                (cl.shaft.is_some() && !stalled_walls.contains(&cl.wall.x)) || (cl.dash && !short_walls.contains(&cl.wall.x))
            });
            if needless {
                continue;
            }
            if dead_wall || blocked.contains(&c.tx) || (caught && dash_jump && !c.needs_shadow) {
                continue;
            }
            let res = self.simulate(*c, s, t, region, 0);
            if matches!(c.recipe, Recipe::Jump { .. }) {
                jumps_tried = true;
                climb_could_help |= !matches!(res, Ok(_) | Err(Fail::Landed { source: false, .. }));
            }
            match res {
                Ok(f) => {
                    let ok = match tier {
                        Tier::Stroll => matches!(f.kind, MoveKind::Walk | MoveKind::Drop | MoveKind::DropThrough | MoveKind::Jump | MoveKind::DoubleJump | MoveKind::WallJump | MoveKind::Behind),
                        Tier::Normal => f.kind != MoveKind::Superdash,
                        Tier::Hurry => true,
                    };
                    if !ok || (c.recipe == Recipe::Climb && !self.robust(*c, s, t, region)) {
                        continue;
                    }
                    found = Some(f);
                    break;
                }
                Err(Fail::Landed { above: true, near: true, source: false }) if c.recipe != Recipe::Climb => blocked.push(c.tx),
                Err(Fail::Landed { source: false, .. }) if matches!(c.recipe, Recipe::Jump { .. }) => caught = true,
                // Caught by another surface on the way up that wall: its other variants would be too.
                Err(Fail::Landed { source: false, .. }) if let Some(cl) = c.climb => dead_walls.push(cl.wall.x),
                Err(Fail::Short) if let Some(cl) = c.climb => short_walls.push(cl.wall.x),
                Err(Fail::Other) if let Some(cl) = c.climb => stalled_walls.push(cl.wall.x),
                Err(_) => {}
            }
        }
        self.candidates = cands;
        found
    }

    /// Does `c` also work from the perturbed starts? Brittle scripts would make the follower
    /// slip whenever it arrives a little differently from the validation run.
    fn robust(&mut self, c: Script, s: &Surface, t: &Surface, region: RectF) -> bool {
        let ok = (1..=PERTURBATIONS).all(|k| self.simulate(c, s, t, region, k).is_ok());
        if !ok {
            self.stats.brittle += 1;
        }
        ok
    }

    /// Candidate scripts for a pair, most preferred first.
    fn candidates_for(&self, tier: Tier, s: &Surface, t: &Surface, out: &mut Vec<Script>) {
        let r = self.reach;
        let ab = self.abilities;
        let p = &self.params;
        let gait = if tier == Tier::Stroll { Gait::Walk } else { Gait::Run };
        let v = p.gait_speed_px(gait);
        let rise = s.y - t.y;
        let gap = (gap_between(s, t) - 2.0 * r.half).max(0.0);
        let dash = r.dash;
        let mk = |recipe, start, dir: Side, takeoff: f32, tx: f32| {
            let back = match start {
                Start::Stand => takeoff,
                _ => s.clamp(takeoff - dir.sign() * self.runup(gait)),
            };
            Script { recipe, start, dir, gait, takeoff, back, tx, ty: t.y, needs_shadow: false, climb: None }
        };
        let sep = separated(s, t);
        let overlap = (s.lo.max(t.lo), s.hi.min(t.hi));
        let below = rise < -r.step;
        // Ledges a descent onto `t` would hit first live between these heights.
        let cover_top = if below { s.y + 1.0 } else { t.y - r.jump };
        let land = |want: f32| self.exposed_x(t, want, cover_top);

        if tier == Tier::Hurry {
            if let Some(dir) = sep
                && let Some(tx) = land(near_x(t, dir, r.half))
            {
                let takeoff = s.clamp(edge_x(s, dir, r.half) - dir.sign() * r.half);
                let band = (s.y - r.height, s.y - r.step);
                if !self.wall_between(dir, takeoff, tx, band, false) {
                    out.push(mk(Recipe::Superdash, Start::Stand, dir, takeoff, tx));
                }
            }
            return;
        }

        // Drop through the floor we stand on onto a surface right below.
        if tier == Tier::Normal && below && s.kind != LedgeKind::Ground && overlap.0 <= overlap.1 {
            let mid = (overlap.0 + overlap.1) * 0.5;
            if let Some(x) = land(mid).filter(|x| (overlap.0..=overlap.1).contains(x)) {
                out.push(mk(Recipe::DropThrough, Start::Stand, Side::Right, x, x));
            }
        }
        // Step behind a drawn line onto the next stretch of the same height: quieter than
        // climbing over it or shadow dashing through, so it is tried first.
        if let Some(dir) = sep
            && rise.abs() <= 1.0
            && gap <= r.half
        {
            let (takeoff, tx) = (edge_x(s, dir, r.half), near_x(t, dir, r.half));
            let band = (s.y - r.height, s.y - 1.0);
            let mut walls = self.walls_between(dir, takeoff, tx).filter(|w| w.y0 < band.1 && band.0 < w.y1).peekable();
            if walls.peek().is_some() && walls.all(|w| w.kind == WallKind::Content) {
                out.push(mk(Recipe::Behind, Start::Stand, dir, takeoff, tx));
            }
        }
        // Walk across / off an edge.
        if rise <= r.step {
            match sep {
                Some(dir) if within(gap, r.walk_off(v, rise)) => {
                    let tx = if below { land(near_x(t, dir, r.half)) } else { Some(near_x(t, dir, r.half)) };
                    if let Some(tx) = tx {
                        out.push(mk(Recipe::Walk, Start::Pass, dir, edge_x(s, dir, r.half), tx));
                    }
                }
                Some(_) => {}
                None if below => {
                    for dir in [Side::Right, Side::Left].into_iter().filter(|&d| s.ledge_end(d)) {
                        let beyond = edge_x(s, dir, r.half) + dir.sign() * (2.0 * r.half + 8.0);
                        let reaches = match dir {
                            Side::Right => t.hi >= beyond - r.walk_off(v, rise),
                            Side::Left => t.lo <= beyond + r.walk_off(v, rise),
                        };
                        if reaches && let Some(tx) = land(beyond) {
                            out.push(mk(Recipe::Walk, Start::Pass, dir, edge_x(s, dir, r.half), tx));
                        }
                    }
                }
                None => {}
            }
        }
        // Jumps.
        let (jdir, jtakeoff, want) = match sep {
            Some(dir) => (dir, edge_x(s, dir, r.half), near_x(t, dir, r.half)),
            None if below => {
                // Under us: jump off the ledge end on the side where the target sticks out
                // most (jumping straight up would just land back here).
                let dir = if t.x1 - s.x1 >= s.x0 - t.x0 { Side::Right } else { Side::Left };
                if !s.ledge_end(dir) {
                    return;
                }
                let edge = edge_x(s, dir, r.half);
                (dir, edge, edge + dir.sign() * (2.0 * r.half + 8.0))
            }
            None if overlap.0 <= overlap.1 => {
                let x = (overlap.0 + overlap.1) * 0.5;
                let dir = if (t.x0 + t.x1) * 0.5 >= x { Side::Right } else { Side::Left };
                (dir, x, x)
            }
            None => {
                let dir = if (t.x0 + t.x1) >= (s.x0 + s.x1) { Side::Right } else { Side::Left };
                (dir, edge_x(s, dir, r.half), near_x(t, dir, r.half))
            }
        };
        let Some(jtx) = land(want) else {
            if tier != Tier::Hurry {
                self.climbs(s, t, &mk, tier == Tier::Normal, out);
            }
            return;
        };
        let jump = |double: bool, dash: bool, low: bool| mk(Recipe::Jump { double, dash, low }, Start::Run, jdir, jtakeoff, jtx);
        // A wall in the path that rises above the move's apex and reaches down to the level
        // of the endpoints cannot be passed (except a window wall by a shadow dash).
        let floor = s.y.max(t.y) - r.step;
        let walled = |apex: f32, shadow: bool| {
            self.walls_between(jdir, jtakeoff, jtx).any(|w| w.y0 < apex && w.y1 >= floor && !(shadow && w.kind != WallKind::Monitor))
        };
        let (single_apex, double_apex) = (s.y - r.jump, s.y - r.jump - r.double);
        let shadow = ab.shadow_dash;
        let single_ok = rise < r.jump - 4.0 && !walled(single_apex, false);
        let single_reaches = single_ok && within(gap, r.single(v, rise));
        if single_reaches {
            out.push(jump(false, false, false));
            if rise > r.step {
                out.push(jump(false, false, true));
            }
        }
        // Strolls climb too (jump, double jump, wall jumps at a walking pace), but never dash.
        let dashes = tier != Tier::Stroll;
        // Escalate only when height, distance or a wall in the way calls for it: a pair that a
        // plain jump fails for other reasons (something else catches the knight) rarely
        // becomes possible with more power, and every candidate costs a simulation.
        //  * double jump: a wall in the way whose top a single jump cannot clear but a double
        //    jump can;
        //  * air dash: a window wall in the way too tall to jump over (shadow dash through).
        let low = s.y.min(t.y);
        let reachable = |w: &Wall| w.y1 > low - r.jump - r.double;
        let clears_double =
            self.walls_between(jdir, jtakeoff, jtx).any(|w| reachable(w) && w.y0 < single_apex + 8.0 && w.y0 > double_apex - 8.0);
        let too_tall =
            shadow && self.walls_between(jdir, jtakeoff, jtx).any(|w| w.kind != WallKind::Monitor && reachable(w) && w.y0 <= double_apex - 8.0);
        if dashes && ab.dash && rise <= r.step && within(gap, dash + r.walk_off(v, rise)) {
            let dir = sep.unwrap_or(jdir);
            let takeoff = edge_x(s, dir, r.half) - dir.sign() * (r.half + 2.0);
            let shadow = ab.shadow_dash && self.wall_between(dir, takeoff, jtx, (s.y - r.height, s.y - r.step), true);
            if shadow || !single_reaches {
                out.push(mk(Recipe::GroundDash, Start::Run, dir, takeoff, jtx));
            }
        }
        let double_ok = ab.double_jump && rise < r.jump + r.double - 4.0;
        let double_reaches = double_ok && within(gap, r.double(v, rise)) && !walled(double_apex, false);
        if double_reaches && (!single_reaches || clears_double) {
            out.push(jump(true, false, false));
        }
        let single_dash = rise < r.jump - 4.0 && !walled(single_apex, shadow);
        if dashes && ab.dash && single_dash && within(gap, r.single(v, rise) + dash) && (!single_reaches || too_tall) {
            out.push(Script { needs_shadow: too_tall, ..jump(false, true, false) });
        }
        let double_dash = double_ok && !walled(double_apex, shadow);
        if dashes && ab.dash && double_dash && within(gap, r.double(v, rise) + dash) && (!double_reaches || too_tall) {
            out.push(Script { needs_shadow: too_tall, ..jump(true, true, false) });
        }
        self.climbs(s, t, &mk, dashes, out);
    }

    /// Wall climbs from `s` up to `t`: for each usable wall (best first), a single-wall climb,
    /// a shaft climb when a facing wall is close, and a dash finish when `t` is far from the
    /// wall line.
    fn climbs(&self, s: &Surface, t: &Surface, mk: &impl Fn(Recipe, Start, Side, f32, f32) -> Script, dashes: bool, out: &mut Vec<Script>) {
        let r = self.reach;
        if !self.abilities.wall_jump || s.y - t.y <= r.step {
            return;
        }
        for (wall, dir, shaft) in self.climb_walls(s, t) {
            let sign = dir.sign();
            let hover = wall.x - sign * (r.half + 0.5);
            // Run in from a little way back (or jump across from the end of `s`).
            let takeoff = s.clamp(hover - sign * 48.0);
            // Over the top when `t` lies beyond the wall line, else step off beside it.
            let over = match dir {
                Side::Right => t.hi > wall.x,
                Side::Left => t.lo < wall.x,
            };
            let want = if over { wall.x + sign * (r.half + 8.0) } else { hover - sign * (r.half + 8.0) };
            // Prefer a landing x nothing sits on top of; but a climb comes in from the side,
            // just above `t`, under such ledges, so a covered target is still worth a try
            // (skipping those, even only for over-the-top climbs, measurably cost reach).
            let tx = self.exposed_x(t, want, t.y - r.jump).unwrap_or_else(|| inset(t, want, r.half));
            let cw = |w: &Wall| ClimbWall { x: w.x, y0: w.y0, y1: w.y1 };
            let far = (tx - hover).abs() > 4.0 * r.half + 32.0;
            let mut push = |shaft: Option<ClimbWall>, dash: bool| {
                let climb = Climb { wall: cw(&wall), shaft, dash, over };
                out.push(Script { climb: Some(climb), ..mk(Recipe::Climb, Start::Run, dir, takeoff, tx) });
            };
            push(None, false);
            if let Some(other) = shaft {
                push(Some(cw(&other)), false);
            }
            if far && dashes && self.abilities.dash {
                push(None, true);
            }
        }
    }

    /// Landing x on `t` nearest `want` (inset from the ends) that no local ledge between
    /// heights `top` and `t.y` covers; `None` when all of `t` is covered (a descent onto it
    /// would always be caught first).
    fn exposed_x(&self, t: &Surface, want: f32, top: f32) -> Option<f32> {
        let h = self.reach.half;
        let covers = |x: f32| self.local.ledges.iter().any(|l| l.y > top && l.y < t.y - 1.0 && l.x0 < x + h && l.x1 > x - h);
        let x = inset(t, want, h);
        if !covers(x) {
            return Some(x);
        }
        let m = (h + 8.0).min((t.hi - t.lo) * 0.5);
        let (lo, hi) = (t.lo + m, t.hi - m);
        self.local
            .ledges
            .iter()
            .filter(|l| l.y > top && l.y < t.y - 1.0)
            .flat_map(|l| [l.x0 - h - 1.0, l.x1 + h + 1.0])
            .filter(|&c| lo <= c && c <= hi && !covers(c))
            .min_by(|a, b| (a - want).abs().total_cmp(&(b - want).abs()))
    }

    /// Does a wall blocking travel in `dir` stand between `a` and `b` (feet x; the body's
    /// leading side sweeps the span) and overlap the height band `(top, bottom)`?
    /// `windows_only`: only walls a shadow dash passes.
    fn wall_between(&self, dir: Side, a: f32, b: f32, (top, bottom): (f32, f32), windows_only: bool) -> bool {
        self.walls_between(dir, a, b).any(|w| w.y0 < bottom && top < w.y1 && (!windows_only || w.kind != WallKind::Monitor))
    }

    /// Local walls blocking travel in `dir` that the body's leading side meets between feet
    /// x `a` and `b`.
    fn walls_between(&self, dir: Side, a: f32, b: f32) -> impl Iterator<Item = &Wall> {
        let face = match dir {
            Side::Right => Facing::Left,
            Side::Left => Facing::Right,
        };
        let lead = dir.sign() * self.reach.half;
        let (lo, hi) = (a.min(b) + lead - 1.0, a.max(b) + lead + 1.0);
        self.local_walls.iter().filter(move |w| w.face == face && w.x > lo && w.x < hi)
    }

    /// Walls that can carry the knight from `s` up to `t`, best first: (wall, direction
    /// towards it, facing wall of a shaft if one is close). Any wall qualifies, wherever it
    /// stands, when
    /// * its bottom is within jump + double jump of `s` and it has the knight's side on `s`
    ///   (or a short hop from the end of `s`),
    /// * its top reaches `t`'s height, or ends close enough below it to jump the rest,
    /// * `t` is within a hop (plus dash) of the wall line.
    ///
    /// Ranked by how much climbing and sideways travel they leave; at most [`MAX_CLIMB_WALLS`].
    fn climb_walls(&self, s: &Surface, t: &Surface) -> Vec<(Wall, Side, Option<Wall>)> {
        let r = self.reach;
        let lift = r.jump + if self.abilities.double_jump { r.double } else { 0.0 };
        let reach_bottom = s.y - lift + r.height * 0.5 + 4.0;
        let finish_rise = 0.8 * lift;
        // From the top of a climb: most of a running jump sideways, plus an air dash.
        let run = self.params.gait_speed_px(Gait::Run);
        let finish_dx = 0.6 * r.single(run, 0.0) + if self.abilities.dash { r.dash } else { 0.0 };
        let mut found: Vec<(f32, Wall, Side)> = Vec::new();
        for w in &self.local_walls {
            if w.y1 < reach_bottom || w.y0 > s.y - r.height * 0.5 || w.y0 - t.y > finish_rise {
                continue;
            }
            // The knight climbs on the wall's face side: left of a Left-facing wall.
            let dir = match w.face {
                Facing::Left => Side::Right,
                Facing::Right => Side::Left,
            };
            let sign = dir.sign();
            let hover = w.x - sign * (r.half + 0.5);
            let on_side = (s.lo - hover) * sign < 0.0 || (s.hi - hover) * sign < 0.0;
            let from_s = (s.lo - hover).max(hover - s.hi).max(0.0);
            let to_t = (t.lo - r.half - w.x).max(w.x - t.hi - r.half).max(0.0);
            if !on_side || from_s > 4.0 * r.half + 32.0 || to_t > finish_dx {
                continue;
            }
            let score = (w.y0 - t.y).max(0.0) * 2.0 + to_t + from_s;
            found.push((score, *w, dir));
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.truncate(MAX_CLIMB_WALLS);
        found
            .into_iter()
            .map(|(_, w, dir)| {
                // A facing wall on the knight's side, overlapping vertically, a shaft away.
                let shaft = self
                    .local_walls
                    .iter()
                    .filter(|o| o.face != w.face && o.y0 < w.y1 && w.y0 < o.y1)
                    .filter(|o| {
                        let gap = (w.x - o.x) * dir.sign();
                        gap >= 2.0 * r.half + 8.0 && gap <= r.jump
                    })
                    .min_by(|a, b| (a.x - w.x).abs().total_cmp(&(b.x - w.x).abs()))
                    .copied();
                (w, dir, shaft)
            })
            .collect()
    }

    fn runup(&self, gait: Gait) -> f32 {
        let p = &self.params;
        let v = p.gait_speed(gait);
        p.px(v * v / (2.0 * p.ground_accel) + 0.5)
    }

    /// Forward-simulate one candidate; `Ok` if it ends standing on `t`.
    ///
    /// `perturb` > 0 varies how the follower might arrive at the take-off (speed within what
    /// the approach accepts, sub-tick phase, a px or two of standing offset), to reject
    /// scripts that only work from one exact start.
    fn simulate(&mut self, mut script: Script, s: &Surface, t: &Surface, region: RectF, perturb: u8) -> Result<Found, Fail> {
        self.stats.sims += 1;
        if script.recipe == Recipe::Climb {
            self.stats.climb_sims += 1;
        }
        let r = self.reach;
        // Per-tick cost grows with the terrain in view, so each run gets only the part of the
        // pair's local terrain its move can reach: its horizontal span and its apex.
        let region = {
            let walls = script.climb.map(|c| [c.wall.x, c.shaft.map_or(c.wall.x, |w| w.x)]);
            let xs = [script.back, script.takeoff, script.tx].into_iter().chain(walls.into_iter().flatten());
            let (lo, hi) = xs.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), x| (lo.min(x), hi.max(x)));
            let slack = r.dash + 2.0 * r.half + 32.0;
            let (apex, mx) = match script.recipe {
                Recipe::Jump { double: true, .. } => (s.y - r.jump - r.double, slack),
                Recipe::Jump { double: false, .. } => (s.y - r.jump, slack),
                Recipe::Climb => (t.y - r.jump, slack + 128.0),
                Recipe::Walk | Recipe::DropThrough | Recipe::GroundDash | Recipe::Superdash | Recipe::Behind => (s.y, slack),
            };
            let top = apex.min(t.y) - 16.0;
            let bottom = s.y.max(t.y) + 1.0;
            RectF::new((lo - mx).max(region.left), top.max(region.top), (hi + mx).min(region.right), bottom)
        };
        copy_around(&self.local, &mut self.sim, region, r);
        let local = &self.sim;
        let p = self.params;
        let v_air = p.gait_speed_px(script.gait);
        let sign = script.dir.sign();
        // With room for a full run-up the follower reaches the take-off at gait speed: start
        // there (a tick early) already running instead of simulating the run-up.
        let full_runup = (script.takeoff - script.back) * sign >= SHORT_RUNUP * self.runup(script.gait);
        let running = script.start != Start::Stand && full_runup;
        let (speed, phase, nudge) = match perturb {
            0 => (1.0, 1.0, 0.0),
            1 => (RUN_FRACTION_MIN, 1.5, -1.5),
            _ => (1.2, 0.35, 1.5),
        };
        let start_x = match script.start {
            Start::Stand => script.takeoff + nudge,
            _ if running => s.clamp(script.takeoff - sign * v_air * speed * SIM_DT * phase),
            _ => script.back,
        };
        // Never start inside a wall: only where the follower could actually stand.
        if start_x < s.lo - 0.5 || start_x > s.hi + 0.5 {
            return Err(Fail::Other);
        }
        let mut ch = Character::new(p, self.abilities, Vec2::new(start_x, s.y));
        ch.facing = script.dir;
        let ev = &mut self.events;
        ev.clear();
        ch.step(&MoveInput::default(), local, SIM_DT, ev);
        if !s.holds(&ch) {
            return Err(Fail::Other);
        }
        if running {
            ch.vel.x = sign * (v_air * speed).min(p.gait_speed_px(Gait::Sprint));
        }
        let mut exec = Exec::new(script);
        let limit = match script.recipe {
            Recipe::Walk => (script.tx - script.takeoff).abs() / v_air + 2.0,
            // About one wall-jump cycle (~1 s) per double-jump height, plus slack.
            Recipe::Climb => 1.5 + (s.y - t.y) / (r.jump + r.double).max(1.0) * 1.5,
            Recipe::Superdash => p.superdash_charge_time + 3.0,
            _ => 4.0,
        };
        let approach_limit = 2.0 * (start_x - script.takeoff).abs() / v_air + 2.0;
        let (mut double, mut wall, mut dash, mut shadow, mut jumped, mut through, mut superdash) =
            (false, false, false, false, false, false, false);
        let (mut hard, mut approach_t, mut still) = (false, 0.0, 0u32);
        let (mut best_y, mut best_t) = (s.y, 0.0f32);
        loop {
            self.stats.ticks += 1;
            if script.recipe == Recipe::Climb {
                self.stats.climb_ticks += 1;
            }
            let input = exec.input(&ch, script.gait, SIM_DT);
            let (old_x, was_shadow) = (ch.pos.x, matches!(ch.state, MoveState::Dashing { shadow: true }));
            ev.clear();
            ch.step(&input, local, SIM_DT, ev);
            let mut shadow_now = was_shadow;
            for e in ev.iter() {
                match *e {
                    MoveEvent::Jumped => jumped = true,
                    MoveEvent::DoubleJumped => double = true,
                    MoveEvent::WallJumped { .. } => wall = true,
                    MoveEvent::DashStarted { shadow } => {
                        dash = true;
                        shadow_now |= shadow;
                    }
                    MoveEvent::DroppedThrough => through = true,
                    MoveEvent::SuperdashLaunched => superdash = true,
                    MoveEvent::Landed { hard: true } => hard = true,
                    _ => {}
                }
            }
            if shadow_now && crosses_window_wall(local, &ch, old_x) {
                shadow = true;
            }
            match exec.outcome(&ch, s, t) {
                Outcome::Done => break,
                Outcome::Wrong => {
                    let near = (ch.pos.x - script.tx).abs() <= ch.size.x;
                    let above = ch.pos.y < t.y;
                    if exec.finished && !above {
                        // A climb that made it to the top but not across.
                        return Err(Fail::Short);
                    }
                    return Err(Fail::Landed { above, near, source: s.shares_ledge(&ch) });
                }
                Outcome::Continue => {}
            }
            if exec.stage == Stage::Approach {
                approach_t += SIM_DT;
                if !s.holds(&ch) || approach_t > approach_limit {
                    return Err(Fail::Other);
                }
                continue;
            }
            if exec.exhausted() && ch.state == MoveState::Airborne && ch.vel.y >= 0.0 {
                // Ballistic from here on: can the target still be reached horizontally?
                let dy = t.y - ch.pos.y;
                let off = (t.x0 - r.half - ch.pos.x).max(ch.pos.x - t.x1 - r.half).max(0.0);
                if dy < -1.0 || off > ch.vel.x.abs().max(v_air) * r.time_to_fall(ch.vel.y, dy) + 4.0 {
                    return Err(Fail::Short);
                }
            }
            still = if (ch.pos.x - old_x).abs() < 0.01 && ch.is_grounded() { still + 1 } else { 0 };
            // Standing still on the ground mid-script (pressed into a wall) never recovers; a
            // superdash charge and turning into the screen are the legitimate pauses.
            let charging = matches!(ch.state, MoveState::SuperdashCharge { .. })
                || ch.behind_phase() == Some(crate::controller::BehindPhase::TurningIn);
            let stuck = still > 12 && !charging;
            let inside = (region.left..=region.right).contains(&ch.pos.x) && (region.top..=region.bottom).contains(&ch.pos.y);
            // A climb that stops gaining height is going nowhere (sliding / hovering in place).
            if script.recipe == Recipe::Climb && exec.stage == Stage::Script {
                if ch.pos.y < best_y - 4.0 {
                    (best_y, best_t) = (ch.pos.y, exec.t);
                } else if exec.t - best_t > CLIMB_STALL {
                    return Err(if exec.finished { Fail::Short } else { Fail::Other });
                }
            }
            if stuck || !inside || exec.t > limit {
                return Err(if exec.finished { Fail::Short } else { Fail::Other });
            }
        }
        let kind = if script.recipe == Recipe::Behind {
            MoveKind::Behind
        } else if superdash {
            MoveKind::Superdash
        } else if shadow {
            MoveKind::ShadowDash
        } else if wall {
            MoveKind::WallJump
        } else if dash {
            MoveKind::Dash
        } else if double {
            MoveKind::DoubleJump
        } else if jumped {
            MoveKind::Jump
        } else if through {
            MoveKind::DropThrough
        } else if exec.airborne {
            MoveKind::Drop
        } else {
            MoveKind::Walk
        };
        script.needs_shadow = shadow;
        let duration = exec.t.max(SIM_DT) + if hard { p.hard_landing_time } else { 0.0 }
            + if kind == MoveKind::Behind { crate::controller::BEHIND_RETURN_TIME } else { 0.0 };
        Ok(Found { kind, duration, land: ch.pos, script })
    }
}

/// Copy into `dst` the part of `src` a body whose feet stay inside `region` can touch.
fn copy_around(src: &Terrain, dst: &mut Terrain, region: RectF, r: Reach) {
    let g = RectF::new(region.left - r.half - 2.0, region.top - r.height - 2.0, region.right + r.half + 2.0, region.bottom + 2.0);
    dst.generation = src.generation;
    dst.bounds = src.bounds;
    dst.windows.clear();
    dst.windows.extend(src.windows.iter().filter(|w| w.rect.overlaps(&g)));
    dst.ledges.clear();
    dst.ledges.extend(src.ledges.iter().filter(|e| e.x0 <= g.right && e.x1 >= g.left && e.y >= g.top && e.y <= g.bottom));
    dst.walls.clear();
    dst.walls.extend(src.walls.iter().filter(|w| w.x >= g.left && w.x <= g.right && w.y0 <= g.bottom && w.y1 >= g.top));
}

/// Did the body just pass (from `old_x`) through a window wall that blocks that direction?
fn crosses_window_wall(t: &Terrain, ch: &Character, old_x: f32) -> bool {
    let half = ch.size.x * 0.5;
    let body = ch.body();
    let (face, a, b) = if ch.pos.x > old_x {
        (Facing::Left, old_x + half, ch.pos.x + half)
    } else {
        (Facing::Right, ch.pos.x - half, old_x - half)
    };
    t.walls.iter().any(|w| {
        w.owner.is_some() && w.face == face && w.x > a - 0.5 && w.x <= b + 0.5 && w.y0 < body.bottom && body.top < w.y1
    })
}

/// Horizontal gap between the surfaces (0 when they overlap).
fn gap_between(s: &Surface, t: &Surface) -> f32 {
    (t.x0 - s.x1).max(s.x0 - t.x1).max(0.0)
}

/// Direction from `s` to `t` when `t` lies entirely to one side.
fn separated(s: &Surface, t: &Surface) -> Option<Side> {
    if t.lo >= s.hi && t.x0 >= s.x1 - 1.0 {
        Some(Side::Right)
    } else if t.hi <= s.lo && t.x1 <= s.x0 + 1.0 {
        Some(Side::Left)
    } else {
        None
    }
}

/// Take-off x at the end of `s` in `dir`. At a cut (where walking on crosses into the next
/// segment) it stays a body half inside, so a running approach does not overshoot onto the
/// neighbour before the script starts.
fn edge_x(s: &Surface, dir: Side, half: f32) -> f32 {
    let inset = if s.ledge_end(dir) { 0.0 } else { half };
    match dir {
        Side::Right => (s.hi - inset).max(s.lo),
        Side::Left => (s.lo + inset).min(s.hi),
    }
}

/// Landing target on `t` just inside its near end when arriving moving in `dir`.
fn near_x(t: &Surface, dir: Side, half: f32) -> f32 {
    let x = match dir {
        Side::Right => t.lo + half + 8.0,
        Side::Left => t.hi - half - 8.0,
    };
    inset(t, x, half)
}

/// `x` clamped into `t`, a body width away from the ends when `t` is wide enough.
fn inset(t: &Surface, x: f32, half: f32) -> f32 {
    let m = (half + 8.0).min((t.hi - t.lo) * 0.5);
    let (lo, hi) = (t.lo + m, t.hi - m);
    // `lo` can round past `hi` on a surface exactly two margins wide.
    if lo < hi { x.clamp(lo, hi) } else { (t.lo + t.hi) * 0.5 }
}
