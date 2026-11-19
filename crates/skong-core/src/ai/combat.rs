//! The knight's combat kit: which of his tools fits the moment in a fight, so he uses the
//! whole moveset (not just the nail) and every move has a reason and a cooldown.
//!
//! * **Pogo**: something right under him mid-air (a foe, Zote, the cursor) → down slash and
//!   bounce off it (`MoveInput::pogo` → `MoveEvent::Pogo`), again on the way down, up to
//!   [`POGO_CHAIN`] bounces, then a rest. A ground foe close by is jumped at to set one up.
//! * **Desolate Dive**: a foe on the ground well below him (beyond a down slash) → dive onto
//!   it (Descending Dark with a fuller soul vessel); the slam hits what is near where he
//!   lands ([`dive_hits`]).
//! * **Dodge**: a foe lunging at him from close → shadow dash through it; he is untouchable
//!   while he passes through.
//! * **Crystal dash opener**: a far foe on his own ledge → charge, fly at it, stop short.
//! * **Wall leap**: sliding down a wall under a flyer → kick off the wall at it.
//! * Spells and nail arts come from the spell book ([`crate::stakes::SpellCues`] tells it
//!   what the foe is up to); Focus heals after a fight.
//!
//! The `*_now` functions are the decisions (pure, unit-tested); [`Kit`] keeps the cooldowns
//! and the moves that last several ticks, and the behaviours (`fight`, `battle`, `pester`)
//! apply the hits.
use crate::controller::{Character, MoveEvent, MoveInput, MoveState, Side};
use crate::geom::Vec2;
use crate::stakes::Spell;
use crate::stakes::spells::SPELL_SOUL;

use super::behaviours::Ctx;
use super::fight::Foe;
use super::stakes::{StakeEvent, Technique};

/// A down slash reaches this far below his feet (units).
pub const DOWN_REACH: f32 = 1.6;
/// Horizontal slack for a pogo past the target's edge (units).
pub const POGO_SLACK: f32 = 0.7;
/// A ground target this close (units, edge to centre) is jumped at to pogo it.
pub const POGO_SETUP: f32 = 2.5;
/// Bounces in one pogo run before he lands and does something else.
pub const POGO_CHAIN: u32 = 4;
/// Seconds between pogo runs.
pub const POGO_REST: f32 = 2.5;
/// The dive's slam hits foes this far (units) beside where he lands.
pub const DIVE_RADIUS: f32 = 2.5;
/// Dives only onto things at most this far below (units).
pub const DIVE_MAX_DROP: f32 = 14.0;
/// Seconds between dives.
pub const DIVE_COOLDOWN: f32 = 8.0;
/// Damage of a dive's slam (Descending Dark: one more).
pub const DIVE_DAMAGE: u32 = 2;
/// A lunge this close (units, edge to centre) is dodged.
pub const DODGE_RANGE: f32 = 3.0;
/// Seconds between dodges.
pub const DODGE_COOLDOWN: f32 = 4.0;
/// Crystal dash openers go at foes this far away (units).
pub const OPENER_RANGE: [f32; 2] = [9.0, 24.0];
/// Seconds between crystal dash openers.
pub const OPENER_COOLDOWN: f32 = 20.0;
/// Seconds between pogo swings (a bounce takes about a second anyway).
pub const POGO_SWING_CD: f32 = 0.25;
/// Seconds between wall leaps.
pub const LEAP_COOLDOWN: f32 = 2.0;

/// A target as the kit sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Mark {
    /// Target centre − knight body centre (px, y down).
    pub off: Vec2,
    /// Half its size (px; the cursor is a point).
    pub half: Vec2,
    pub flying: bool,
    pub winding_up: bool,
    pub lunging: bool,
}

impl Mark {
    pub fn foe(f: &Foe, centre: Vec2) -> Self {
        Self { off: f.pos - centre, half: f.half, flying: f.flying, winding_up: f.winding_up, lunging: f.lunging }
    }

    /// The cursor (`off` from his body centre).
    pub fn point(off: Vec2) -> Self {
        Self { off, ..Default::default() }
    }

    /// Its top, relative to his body centre (px, y down).
    fn top(&self) -> f32 {
        self.off.y - self.half.y
    }
}

/// The knight, as the decisions need him.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Stance {
    pub ppu: f32,
    /// Half his body height (px).
    pub half_h: f32,
    pub grounded: bool,
    /// Plain airborne (jumping / falling; not dashing, diving, on a wall).
    pub airborne: bool,
    /// Vertical speed (px/s, y down).
    pub vy: f32,
    pub wall_slide: bool,
    /// Standing on a one-way ledge (a dive goes through it).
    pub on_one_way: bool,
    pub can_dash: bool,
    pub shadow_ready: bool,
    pub superdash: bool,
    pub dive: bool,
}

impl Stance {
    pub fn of(k: &Character) -> Self {
        Self {
            ppu: k.params.px_per_unit,
            half_h: 0.5 * k.size.y,
            grounded: k.is_grounded(),
            airborne: k.state == MoveState::Airborne,
            vy: k.vel.y,
            wall_slide: matches!(k.state, MoveState::WallSlide { .. }),
            on_one_way: k.state == MoveState::Grounded && k.support.is_some_and(|s| s.kind.is_one_way()),
            can_dash: k.can_dash(),
            shadow_ready: k.shadow_dash_ready(),
            superdash: k.abilities.superdash,
            dive: k.abilities.dive,
        }
    }
}

/// Pogo: mid-air, at the top of a jump or coming down, with the target right under his feet
/// within a down slash's reach.
pub(crate) fn pogo_now(m: &Mark, s: &Stance) -> bool {
    let top = m.top();
    s.airborne
        && s.vy >= -s.ppu
        && top >= -0.3 * s.half_h
        && top <= s.half_h + DOWN_REACH * s.ppu
        && m.off.x.abs() <= m.half.x + POGO_SLACK * s.ppu
}

/// Set up a pogo: standing next to a target on his level that does not fly (it stays put
/// under him): jump at it.
pub(crate) fn pogo_setup(m: &Mark, s: &Stance) -> bool {
    s.grounded && !m.flying && m.off.x.abs() <= m.half.x + POGO_SETUP * s.ppu && m.off.y.abs() <= m.half.y + s.half_h + 0.5 * s.ppu
}

/// Dive: a target on the ground well below him (out of a down slash's reach), nearly
/// straight down. In the air, or standing on a one-way ledge (the dive goes through).
pub(crate) fn dive_now(m: &Mark, s: &Stance) -> bool {
    let top = m.top();
    s.dive
        && (s.airborne || s.on_one_way)
        && !m.flying
        && top > s.half_h + (DOWN_REACH + 0.5) * s.ppu
        && top < DIVE_MAX_DROP * s.ppu
        && m.off.x.abs() <= m.half.x + 0.6 * s.ppu
}

/// Does a dive that slammed down at `land` (his feet) hit `f`?
pub(crate) fn dive_hits(land: Vec2, f: &Foe, ppu: f32) -> bool {
    let beside = (f.pos.x - land.x).abs() <= f.half.x + DIVE_RADIUS * ppu;
    // Its body overlaps the slam's band (from the ground up to about his height).
    let band = (land.y - 2.0 * ppu, land.y + 0.5 * ppu);
    beside && f.pos.y + f.half.y >= band.0 && f.pos.y - f.half.y <= band.1
}

/// Dodge: a foe lunging at him from close on his level → shadow dash through it (the side
/// to dash to).
pub(crate) fn dodge_now(m: &Mark, s: &Stance) -> Option<Side> {
    let close = m.off.x.abs() <= m.half.x + DODGE_RANGE * s.ppu && m.off.y.abs() <= m.half.y + s.half_h;
    (m.lunging && close && s.grounded && s.can_dash && s.shadow_ready).then(|| Side::of(m.off.x)).flatten()
}

/// Crystal dash opener: a far foe on his own ledge (`same_ledge`), level with him.
pub(crate) fn opener_now(m: &Mark, s: &Stance, same_ledge: bool) -> bool {
    let d = m.off.x.abs() / s.ppu;
    s.superdash && s.grounded && same_ledge && !m.flying && m.off.y.abs() <= m.half.y + s.half_h && (OPENER_RANGE[0]..=OPENER_RANGE[1]).contains(&d)
}

/// Wall leap: sliding down a wall with the target above him.
pub(crate) fn wall_leap_now(m: &Mark, s: &Stance) -> bool {
    s.wall_slide && m.off.y < -(m.half.y + s.half_h)
}

/// Foes a Cyclone Slash would catch (a crowd of two or more makes him spin).
pub(crate) fn crowd(foes: &[Foe], centre: Vec2, scale: f32) -> u32 {
    foes.iter().filter(|f| f.vulnerable && Spell::Cyclone.reaches(f.pos - centre, scale)).count() as u32
}

/// The crystal dash opener in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Opener {
    /// Holding the button (seconds held).
    Charging(f32),
    /// Flying at the foe (seconds since the launch).
    Flying(f32),
}

/// The kit's state in one fight: cooldowns (brain clock) and the moves that span ticks.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Kit {
    /// Bounces in the current pogo run.
    pub chain: u32,
    /// Jumped to pogo: stay over the target.
    pub pogo_run: bool,
    pogo_jumped_at: f32,
    pub pogo_ready_at: f32,
    /// A dive is on its way down (since this time): strike where it lands.
    diving: Option<f32>,
    pub dive_ready_at: f32,
    pub dodge_ready_at: f32,
    /// Shadow dashing through a foe: its bumps don't count until then.
    pub untouchable_until: f32,
    opener: Option<Opener>,
    pub opener_ready_at: f32,
    pub leap_ready_at: f32,
}

/// What the kit did with this tick's input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum KitMove {
    /// A down slash at the target under him (the caller decides whether it connects and
    /// then calls [`Kit::bounce`]).
    PogoSwing,
    /// Steering over the target between bounces.
    PogoSteer,
    /// Jumped at a target to pogo it.
    PogoJump,
    Dive { dark: bool },
    Dodge(Side),
    /// Charging or flying a crystal dash at the target.
    Opening,
    WallLeap,
    /// A dive or a superdash is under way: hands off.
    Busy,
}

impl Kit {
    pub fn new(now: f32) -> Self {
        // A fresh fight: openers allowed, a first pogo run right away.
        Self { pogo_ready_at: now, ..Default::default() }
    }

    /// Bumps that land now count (not while shadow dashing through a foe).
    pub fn touchable(&self, now: f32) -> bool {
        now >= self.untouchable_until
    }

    /// May he bounce now (on a pogo run, or rested since the last one)?
    pub fn may_pogo(&self, now: f32) -> bool {
        self.pogo_run || now >= self.pogo_ready_at
    }

    /// The down slash connected: bounce (sets `input.pogo`), and end the run after
    /// [`POGO_CHAIN`] bounces (no more until he has rested).
    pub fn bounce(&mut self, now: f32, input: &mut MoveInput) {
        if !self.may_pogo(now) {
            return;
        }
        input.pogo = true;
        self.chain += 1;
        if self.chain >= POGO_CHAIN {
            self.end_pogo(now);
        }
    }

    fn end_pogo(&mut self, now: f32) {
        if self.pogo_run || self.chain > 0 {
            self.pogo_ready_at = now + POGO_REST;
        }
        self.pogo_run = false;
        self.chain = 0;
    }

    /// A dive he started is over: did it just slam down (feet at `land`)? Call every tick.
    pub fn dive_landed(&mut self, ctx: &Ctx) -> Option<Vec2> {
        let since = self.diving?;
        let k = ctx.p.knight;
        if ctx.p.move_events.contains(&MoveEvent::DiveLanded) {
            self.diving = None;
            return Some(k.pos);
        }
        if ctx.now - since > 0.3 && !matches!(k.state, MoveState::Dive { .. }) {
            self.diving = None; // never got going, or ended without a slam
        }
        None
    }

    /// The kit's move for this tick against `m`, if one fits (it fills `input`).
    /// `attack_ready`: the nail is ready to swing; `same_ledge`: the target is on his ledge
    /// (a crystal dash can reach it); `may_pogo_setup`: jumping at it is a good idea now.
    #[allow(clippy::too_many_arguments)]
    pub fn step(&mut self, ctx: &mut Ctx, m: &Mark, attack_ready: bool, same_ledge: bool, may_pogo_setup: bool, input: &mut MoveInput) -> Option<KitMove> {
        let k = ctx.p.knight;
        let s = Stance::of(k);
        let now = ctx.now;
        let ppu = s.ppu;
        if matches!(k.state, MoveState::Dive { .. }) || self.diving.is_some() && !s.grounded {
            return Some(KitMove::Busy);
        }
        if let Some(mv) = self.opening(ctx, m, &s, input) {
            return Some(mv);
        }
        if s.grounded && self.chain > 0 {
            // Landed between bounces: that run is over.
            self.end_pogo(now);
        }
        if s.grounded && self.pogo_run && now - self.pogo_jumped_at > 0.3 {
            // Back on the ground (or the jump never happened): the run is over.
            self.end_pogo(now);
        }

        if let Some(mv) = self.dodge(ctx, m, input) {
            return Some(mv);
        }
        // (Not out of his own pogo jump: that one is for bouncing.)
        if !self.pogo_run && now >= self.dive_ready_at && ctx.stakes.spells.soul + 1e-4 >= SPELL_SOUL && dive_now(m, &s) {
            let dark = ctx.stakes.spells.soul >= 2.0 * SPELL_SOUL - 1e-4;
            ctx.stakes.spells.soul = (ctx.stakes.spells.soul - SPELL_SOUL).max(0.0);
            input.dive_pressed = true;
            input.dive_dark = dark;
            self.diving = Some(now);
            self.dive_ready_at = now + DIVE_COOLDOWN;
            self.end_pogo(now);
            ctx.out.stakes.push(StakeEvent::Technique(Technique::Dive { dark }));
            log::debug!("brain: {} onto the foe below", if dark { "descending dark" } else { "desolate dive" });
            return Some(KitMove::Dive { dark });
        }
        if attack_ready && self.may_pogo(now) && pogo_now(m, &s) {
            input.x = (m.off.x / (0.5 * ppu)).clamp(-1.0, 1.0);
            return Some(KitMove::PogoSwing);
        }
        if self.pogo_run && !s.grounded {
            // Stay right over it between bounces.
            input.x = (m.off.x / ppu).clamp(-1.0, 1.0);
            return Some(KitMove::PogoSteer);
        }
        if now >= self.leap_ready_at && wall_leap_now(m, &s) {
            input.jump_pressed = true;
            input.jump_held = true;
            self.leap_ready_at = now + LEAP_COOLDOWN;
            ctx.out.stakes.push(StakeEvent::Technique(Technique::WallLeap));
            return Some(KitMove::WallLeap);
        }
        if now >= self.opener_ready_at && opener_now(m, &s, same_ledge) {
            self.opener = Some(Opener::Charging(0.0));
            self.opener_ready_at = now + OPENER_COOLDOWN;
            ctx.out.stakes.push(StakeEvent::Technique(Technique::CrystalOpener));
            log::debug!("brain: crystal dash at the far foe");
            return self.opening(ctx, m, &s, input);
        }
        if may_pogo_setup && now >= self.pogo_ready_at && pogo_setup(m, &s) {
            input.x = (m.off.x / ppu).clamp(-1.0, 1.0);
            input.jump_pressed = true;
            input.jump_held = true;
            self.pogo_run = true;
            self.pogo_jumped_at = now;
            self.chain = 0;
            return Some(KitMove::PogoJump);
        }
        None
    }

    /// Dodge `m` (lunging at him from close) with a shadow dash through it, if one is ready;
    /// he is untouchable while he passes through.
    pub fn dodge(&mut self, ctx: &mut Ctx, m: &Mark, input: &mut MoveInput) -> Option<KitMove> {
        let k = ctx.p.knight;
        let now = ctx.now;
        if now < self.dodge_ready_at {
            return None;
        }
        let side = dodge_now(m, &Stance::of(k))?;
        input.x = side.sign();
        input.dash_pressed = true;
        self.dodge_ready_at = now + DODGE_COOLDOWN;
        self.untouchable_until = now + k.params.dash_time + 0.2;
        self.end_pogo(now);
        ctx.out.stakes.push(StakeEvent::Technique(Technique::Dodge));
        log::debug!("brain: shadow dash through the lunge");
        Some(KitMove::Dodge(side))
    }

    /// The crystal dash opener under way: charge facing the foe, launch, stop short of it.
    fn opening(&mut self, ctx: &Ctx, m: &Mark, s: &Stance, input: &mut MoveInput) -> Option<KitMove> {
        let k = ctx.p.knight;
        let side = Side::of(m.off.x).unwrap_or(k.facing);
        match self.opener? {
            Opener::Charging(t) => {
                let t = t + ctx.dt();
                // Lost footing, or it came at him meanwhile: let go (an uncharged release
                // just cancels).
                let close = m.off.x.abs() < 0.5 * OPENER_RANGE[0] * s.ppu;
                if !s.grounded && !matches!(k.state, MoveState::SuperdashCharge { .. }) || close {
                    self.opener = None;
                    return None;
                }
                if t < k.params.superdash_charge_time + 0.05 {
                    input.superdash_held = true;
                    input.x = side.sign();
                    self.opener = Some(Opener::Charging(t));
                } else {
                    self.opener = Some(Opener::Flying(0.0));
                }
                Some(KitMove::Opening)
            }
            Opener::Flying(t) => {
                let t = t + ctx.dt();
                if k.state != MoveState::Superdash {
                    if t > 0.1 {
                        self.opener = None;
                        return None;
                    }
                    self.opener = Some(Opener::Flying(t));
                    return Some(KitMove::Opening);
                }
                self.opener = Some(Opener::Flying(t));
                // Stop just short of it (or once past it): jump cancels the flight.
                let short = m.half.x + 1.5 * s.ppu;
                if m.off.x.abs() < short || Side::of(m.off.x) != Some(k.facing) {
                    input.jump_pressed = true;
                }
                Some(KitMove::Opening)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PPU: f32 = 64.0;

    fn air(vy: f32) -> Stance {
        Stance { ppu: PPU, half_h: 41.0, airborne: true, vy, can_dash: true, shadow_ready: true, superdash: true, dive: true, ..Default::default() }
    }

    fn ground() -> Stance {
        Stance { airborne: false, grounded: true, vy: 0.0, ..air(0.0) }
    }

    fn crawlid(dx: f32, dy: f32) -> Mark {
        Mark { off: Vec2::new(dx, dy), half: Vec2::new(51.0, 38.0), ..Default::default() }
    }

    #[test]
    fn pogo_only_when_it_is_right_under_him_on_the_way_down() {
        // Its top 60 px under his centre (his feet are 41 px under it): in reach.
        assert!(pogo_now(&crawlid(10.0, 98.0), &air(200.0)));
        assert!(pogo_now(&crawlid(10.0, 98.0), &air(-20.0)), "at the apex");
        assert!(!pogo_now(&crawlid(10.0, 98.0), &air(-500.0)), "not while shooting up");
        assert!(!pogo_now(&crawlid(10.0, 98.0), &ground()), "not standing");
        assert!(!pogo_now(&crawlid(150.0, 98.0), &air(200.0)), "not beside it");
        assert!(!pogo_now(&crawlid(0.0, 400.0), &air(200.0)), "too far below");
        assert!(!pogo_now(&crawlid(0.0, -100.0), &air(200.0)), "not above him");
        // The cursor is a point.
        assert!(pogo_now(&Mark::point(Vec2::new(5.0, 80.0)), &air(100.0)));
    }

    #[test]
    fn pogo_setup_jumps_at_close_ground_targets_only() {
        assert!(pogo_setup(&crawlid(120.0, 0.0), &ground()));
        assert!(!pogo_setup(&crawlid(500.0, 0.0), &ground()), "too far");
        assert!(!pogo_setup(&Mark { flying: true, ..crawlid(60.0, 0.0) }, &ground()), "flyers move off");
        assert!(!pogo_setup(&crawlid(60.0, 0.0), &air(0.0)), "already up");
    }

    #[test]
    fn dives_onto_ground_foes_far_below() {
        assert!(dive_now(&crawlid(10.0, 400.0), &air(100.0)));
        assert!(!dive_now(&crawlid(10.0, 98.0), &air(100.0)), "a down slash does: pogo");
        assert!(!dive_now(&crawlid(300.0, 400.0), &air(100.0)), "not straight below");
        assert!(!dive_now(&Mark { flying: true, ..crawlid(0.0, 400.0) }, &air(100.0)), "flyers dodge it");
        assert!(!dive_now(&crawlid(0.0, 400.0), &ground()), "not from solid ground");
        assert!(dive_now(&crawlid(0.0, 400.0), &Stance { on_one_way: true, ..ground() }), "through a window top");
        let f = Foe { pos: Vec2::new(1000.0, 1000.0), feet: Vec2::new(1000.0, 1040.0), half: Vec2::new(50.0, 40.0), ..Default::default() };
        assert!(dive_hits(Vec2::new(1100.0, 1040.0), &f, PPU));
        assert!(!dive_hits(Vec2::new(1400.0, 1040.0), &f, PPU), "too far beside");
        assert!(!dive_hits(Vec2::new(1000.0, 600.0), &f, PPU), "landed on a ledge above it");
    }

    #[test]
    fn dodges_a_close_lunge_with_a_shadow_dash_through_it() {
        let lunge = Mark { lunging: true, ..crawlid(-150.0, 0.0) };
        assert_eq!(dodge_now(&lunge, &ground()), Some(Side::Left), "through it");
        assert_eq!(dodge_now(&crawlid(-150.0, 0.0), &ground()), None, "no lunge");
        assert_eq!(dodge_now(&Mark { lunging: true, ..crawlid(-600.0, 0.0) }, &ground()), None, "far off");
        assert_eq!(dodge_now(&lunge, &Stance { shadow_ready: false, ..ground() }), None, "shadow dash cooling down");
    }

    #[test]
    fn opens_with_a_crystal_dash_at_far_foes_on_his_ledge() {
        assert!(opener_now(&crawlid(800.0, 0.0), &ground(), true));
        assert!(!opener_now(&crawlid(800.0, 0.0), &ground(), false), "another ledge");
        assert!(!opener_now(&crawlid(300.0, 0.0), &ground(), true), "close: just walk");
        assert!(!opener_now(&crawlid(800.0, -300.0), &ground(), true), "not level");
    }

    #[test]
    fn wall_leaps_at_things_above_and_crowds_make_him_spin() {
        let flyer = Mark { flying: true, ..crawlid(100.0, -300.0) };
        assert!(wall_leap_now(&flyer, &Stance { wall_slide: true, airborne: false, ..air(500.0) }));
        assert!(!wall_leap_now(&flyer, &air(0.0)), "only off a wall");
        let foe = |x: f32| Foe { pos: Vec2::new(x, 0.0), vulnerable: true, ..Default::default() };
        assert_eq!(crowd(&[foe(60.0), foe(-80.0), foe(900.0)], Vec2::ZERO, 1.0), 2);
    }
}
