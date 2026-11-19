//! Who stands where: one shared registry of the screen rects taken by props (placed
//! benches, things found in windows) and other anchored things (NPCs, the Shade, ...), so
//! that nothing is placed inside something else.
//!
//! Every placement asks [`Occupancy::is_free`] (or [`Occupancy::nudge`] for the nearest
//! free spot along a ledge) and then [`Occupancy::claim`]s its rect under a stable
//! [`ClaimKey`]. Claims are replaced by key, so owners simply re-claim whenever their rect
//! moves and [`Occupancy::release`] when they go. Loose things (Geo coins) only ask
//! [`Occupancy::covering`] so they don't come to rest inside a prop.
use crate::geom::{RectF, Vec2};

/// What holds a claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ClaimKind {
    /// A bench placed by the user (tray / panel / hotkey).
    Bench,
    /// Something found in a window (`crate::discovery`).
    Find,
    /// A visiting NPC's spot.
    Npc,
    /// The Shade's resting place.
    Shade,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClaimKey {
    pub kind: ClaimKind,
    pub id: u64,
}

impl ClaimKey {
    pub const fn new(kind: ClaimKind, id: u64) -> Self {
        Self { kind, id }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Claim {
    pub key: ClaimKey,
    pub rect: RectF,
    /// Higher wins a collision (the loser moves aside or hides).
    pub priority: i32,
}

/// Claims never touch closer than this (px): props side by side keep a small gap.
pub const GAP: f32 = 6.0;

#[derive(Clone, Debug, Default)]
pub struct Occupancy {
    claims: Vec<Claim>,
}

impl Occupancy {
    pub fn claims(&self) -> &[Claim] {
        &self.claims
    }

    pub fn get(&self, key: ClaimKey) -> Option<&Claim> {
        self.claims.iter().find(|c| c.key == key)
    }

    /// Take `rect` (replacing this key's previous claim).
    pub fn claim(&mut self, key: ClaimKey, rect: RectF, priority: i32) {
        match self.claims.iter_mut().find(|c| c.key == key) {
            Some(c) => {
                c.rect = rect;
                c.priority = priority;
            }
            None => self.claims.push(Claim { key, rect, priority }),
        }
    }

    pub fn release(&mut self, key: ClaimKey) {
        self.claims.retain(|c| c.key != key);
    }

    /// Drop every claim of `kind` (an owner about to re-claim all of its rects).
    pub fn release_kind(&mut self, kind: ClaimKind) {
        self.claims.retain(|c| c.key.kind != kind);
    }

    /// Claims overlapping `rect` (with the [`GAP`]), other than `except`.
    pub fn blockers(&self, rect: RectF, except: Option<ClaimKey>) -> impl Iterator<Item = &Claim> {
        let r = RectF::new(rect.left - GAP, rect.top, rect.right + GAP, rect.bottom);
        self.claims.iter().filter(move |c| Some(c.key) != except && c.rect.overlaps(&r))
    }

    pub fn is_free(&self, rect: RectF, except: Option<ClaimKey>) -> bool {
        self.blockers(rect, except).next().is_none()
    }

    /// Is `rect` free of claims with priority >= `priority` (lower ones would yield)?
    pub fn is_free_above(&self, rect: RectF, except: Option<ClaimKey>, priority: i32) -> bool {
        !self.blockers(rect, except).any(|c| c.priority >= priority)
    }

    /// Smallest horizontal shift that makes `rect` free while it stays within `[x0, x1]`
    /// (a ledge span), searched in `step` px increments. `Some(0.0)` when already free.
    pub fn nudge(&self, rect: RectF, x0: f32, x1: f32, except: Option<ClaimKey>, step: f32) -> Option<f32> {
        let step = step.max(1.0);
        let lo = x0 - rect.left; // most negative shift allowed
        let hi = x1 - rect.right; // most positive shift allowed
        if lo > hi {
            return None;
        }
        let shifted = |dx: f32| RectF::new(rect.left + dx, rect.top, rect.right + dx, rect.bottom);
        let n = ((hi - lo) / step).ceil() as i32 + 1;
        for i in 0..=n {
            for dx in [i as f32 * step, -(i as f32) * step] {
                if dx >= lo && dx <= hi && self.is_free(shifted(dx), except) {
                    return Some(dx);
                }
            }
        }
        None
    }

    /// The claim whose rect (shrunk by `inset`) contains `p`: loose things must not rest there.
    pub fn covering(&self, p: Vec2, inset: f32) -> Option<&Claim> {
        self.claims.iter().find(|c| {
            let r = c.rect;
            p.x > r.left + inset && p.x < r.right - inset && p.y > r.top + inset && p.y <= r.bottom + 1.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x0: f32, x1: f32) -> RectF {
        RectF::new(x0, 50.0, x1, 100.0)
    }

    #[test]
    fn claims_block_and_release() {
        let mut o = Occupancy::default();
        let a = ClaimKey::new(ClaimKind::Bench, 1);
        o.claim(a, r(100.0, 200.0), 10);
        assert!(!o.is_free(r(150.0, 250.0), None));
        assert!(o.is_free(r(150.0, 250.0), Some(a)), "own claim does not block");
        assert!(o.is_free(r(220.0, 300.0), None));
        assert!(!o.is_free(r(203.0, 300.0), None), "keeps a small gap");
        o.claim(a, r(400.0, 500.0), 10);
        assert!(o.is_free(r(150.0, 250.0), None), "re-claim moves it");
        o.release(a);
        assert!(o.claims().is_empty());
    }

    #[test]
    fn nudges_to_the_nearest_free_spot_on_the_ledge() {
        let mut o = Occupancy::default();
        o.claim(ClaimKey::new(ClaimKind::Find, 1), r(100.0, 200.0), 10);
        let dx = o.nudge(r(180.0, 230.0), 0.0, 1000.0, None, 4.0).expect("room");
        assert!(dx > 0.0 && dx <= 32.0, "shifted right past it: {dx}");
        assert_eq!(o.nudge(r(120.0, 170.0), 90.0, 210.0, None, 4.0), None, "no room on a short ledge");
        assert_eq!(o.nudge(r(300.0, 350.0), 0.0, 1000.0, None, 4.0), Some(0.0));
    }

    #[test]
    fn coins_may_touch_but_not_rest_inside() {
        let mut o = Occupancy::default();
        o.claim(ClaimKey::new(ClaimKind::Bench, 1), r(100.0, 200.0), 10);
        assert!(o.covering(Vec2::new(150.0, 100.0), 4.0).is_some());
        assert!(o.covering(Vec2::new(102.0, 100.0), 4.0).is_none(), "touching the edge is fine");
        assert!(o.covering(Vec2::new(250.0, 100.0), 4.0).is_none());
    }
}
