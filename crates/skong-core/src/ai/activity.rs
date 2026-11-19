//! Screen activity: where things change on screen, the knight's main source of curiosity.
//!
//! [`Activity`] / [`ActivitySpot`] mirror the shape the vision layer exposes
//! (`skong_core::vision::Activity` via `ContentFeed::activity()`). Until that lands they
//! are defined here; once it does, replace both with
//! `pub use crate::vision::{Activity, ActivitySpot};` (same fields).
//!
//! The [`ActivityTracker`] gives spots an identity across frames (spots are re-detected
//! every frame with slightly different rects), remembers how long each has been quiet and
//! how *satiated* the knight is with it, so he watches a spot for a while and then moves on.
use crate::desktop::WindowId;
use crate::geom::{RectI, Vec2};

/// One region of the screen that is changing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActivitySpot {
    pub rect: RectI,
    pub center: Vec2,
    /// 0..1: how much / how strongly it changes.
    pub intensity: f32,
    /// Seconds it has been active.
    pub age_secs: f32,
    /// Window the spot lies in, if any.
    pub owner: Option<WindowId>,
}

/// All active spots this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activity {
    pub spots: Vec<ActivitySpot>,
}

/// Below this intensity a spot counts as quiet.
pub const QUIET: f32 = 0.12;
/// Satiation half-life while not being watched (s).
const SATIATION_HALF_LIFE: f32 = 180.0;
/// Forget a spot unseen for this long (s).
const FORGET_SECS: f32 = 30.0;

/// A spot with identity and the knight's feelings about it.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackedSpot {
    pub id: u32,
    pub rect: RectI,
    pub center: Vec2,
    pub intensity: f32,
    pub age: f32,
    pub owner: Option<WindowId>,
    /// Seconds below [`QUIET`] (or unseen).
    pub quiet_for: f32,
    /// 0..1: 1 = seen enough of it.
    pub satiation: f32,
}

impl TrackedSpot {
    pub fn active(&self) -> bool {
        self.quiet_for <= 0.0 && self.intensity >= QUIET
    }

    /// Spot still worth watching (allows short lulls).
    pub fn lively(&self) -> bool {
        self.quiet_for < 1.5
    }
}

#[derive(Clone, Debug, Default)]
pub struct ActivityTracker {
    spots: Vec<TrackedSpot>,
    next_id: u32,
}

impl ActivityTracker {
    pub(crate) fn update(&mut self, activity: &Activity, dt: f32) {
        let decay = 0.5f32.powf(dt / SATIATION_HALF_LIFE);
        let mut seen = vec![false; self.spots.len()];
        for s in &activity.spots {
            let reach = 120.0f32.max(0.5 * Vec2::new(s.rect.width() as f32, s.rect.height() as f32).length());
            let matched = self
                .spots
                .iter()
                .enumerate()
                .filter(|(i, t)| !seen[*i] && (t.owner == s.owner || t.owner.is_none() || s.owner.is_none()))
                .map(|(i, t)| (i, t.center.distance(s.center)))
                .filter(|(_, d)| *d <= reach)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            let quiet = s.intensity < QUIET;
            match matched {
                Some(i) => {
                    seen[i] = true;
                    let t = &mut self.spots[i];
                    t.rect = s.rect;
                    t.center = s.center;
                    t.intensity = s.intensity;
                    t.age = s.age_secs;
                    t.owner = s.owner;
                    t.quiet_for = if quiet { t.quiet_for + dt } else { 0.0 };
                }
                None => {
                    self.next_id += 1;
                    self.spots.push(TrackedSpot {
                        id: self.next_id,
                        rect: s.rect,
                        center: s.center,
                        intensity: s.intensity,
                        age: s.age_secs,
                        owner: s.owner,
                        quiet_for: if quiet { dt } else { 0.0 },
                        satiation: 0.0,
                    });
                    seen.push(true);
                }
            }
        }
        for (t, seen) in self.spots.iter_mut().zip(seen) {
            if !seen {
                t.quiet_for += dt;
                t.intensity *= 0.5f32.powf(dt / 0.5);
            }
            t.satiation *= decay;
        }
        self.spots.retain(|t| t.quiet_for < FORGET_SECS || t.satiation > 0.05);
    }

    pub fn spots(&self) -> &[TrackedSpot] {
        &self.spots
    }

    pub fn get(&self, id: u32) -> Option<&TrackedSpot> {
        self.spots.iter().find(|t| t.id == id)
    }

    /// The knight watched spot `id` for a bit.
    pub(crate) fn satiate(&mut self, id: u32, amount: f32) {
        if let Some(t) = self.spots.iter_mut().find(|t| t.id == id) {
            t.satiation = (t.satiation + amount).clamp(0.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(x: i32, intensity: f32) -> ActivitySpot {
        let rect = RectI::new(x, 100, x + 100, 200);
        ActivitySpot { rect, center: Vec2::new(x as f32 + 50.0, 150.0), intensity, age_secs: 1.0, owner: Some(1) }
    }

    #[test]
    fn spots_keep_identity_across_frames() {
        let mut t = ActivityTracker::default();
        t.update(&Activity { spots: vec![spot(100, 0.8)] }, 0.1);
        let id = t.spots()[0].id;
        t.update(&Activity { spots: vec![spot(130, 0.7)] }, 0.1);
        assert_eq!(t.spots().len(), 1);
        assert_eq!(t.spots()[0].id, id);
        t.update(&Activity { spots: vec![spot(130, 0.7), spot(900, 0.5)] }, 0.1);
        assert_eq!(t.spots().len(), 2);
    }

    #[test]
    fn quiet_spots_fade_and_are_forgotten() {
        let mut t = ActivityTracker::default();
        t.update(&Activity { spots: vec![spot(100, 0.8)] }, 0.1);
        for _ in 0..20 {
            t.update(&Activity::default(), 0.1);
        }
        assert!(!t.spots()[0].lively());
        for _ in 0..400 {
            t.update(&Activity::default(), 0.1);
        }
        assert!(t.spots().is_empty());
    }
}
