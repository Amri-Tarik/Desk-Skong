//! Curiosity about highlighted text (on by default; the app's "Curious about highlighted text"
//! toggle). The app reports only *where* the user's text selection is on screen
//! ([`Percepts::selection`]: a rectangle, never the text). A fresh selection that has
//! settled (the user stopped dragging) catches his eye: "?" (the app's emote on the switch),
//! he goes to a vantage point near it (the edge above, a ledge in front of it, or the
//! ledge below) and looks at it (`look_down` / `gaze_into_screen` / `look_up`) once or
//! twice, then gets on with his day.
//!
//! An Ambient pastime: above idling and keeping company, below Geo, visitors and fights
//! (the Engagement tier). Each selection is looked at once, and after a look he leaves
//! selections alone for a while, so selecting text all day doesn't keep him running about.
use crate::geom::RectI;
use crate::nav::NavGoal;

use super::Brain;
use super::behaviours::{ActWait, Active, Beat, Ctx, Status, Travel, TravelStep, style_for};
use super::interest::PoiTarget;
use super::navigator::Navigator;
use super::percept::Percepts;
use super::spots::{self, Vantage, View};
use super::watch::VANTAGE_TOL;

/// A selection must hold still this long before it counts (the user is still dragging).
pub const SETTLE_SECS: f32 = 0.6;
/// A selection older than this is old news (unless he is already on his way).
pub const STALE_SECS: f32 = 25.0;
/// After a look, new selections are ignored this long.
pub const COOLDOWN_SECS: f32 = 30.0;
/// Utility ceiling: a pastime (keeping company tops out near 0.6; Geo and visitors are a
/// tier above anyway).
const MAX_UTILITY: f32 = 0.8;
/// Smallest selection worth a look (px).
const MIN_SIDE: i32 = 2;

/// The selection the knight knows about.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Seen {
    /// Identity: bumped for every new selection (an extended one keeps its id).
    id: u32,
    rect: RectI,
    /// Seconds since it appeared.
    age: f32,
    /// Seconds since its rectangle last changed.
    still: f32,
    /// Already looked at.
    looked: bool,
}

/// The best way to go and look at the current selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HighlightChoice {
    pub id: u32,
    pub vantage: Vantage,
    pub eta: f32,
}

/// What the knight knows about the user's text selection (lives in the interest map). Only
/// a rectangle and a few timers: no text is ever seen.
#[derive(Clone, Debug, Default)]
pub struct HighlightMemory {
    seen: Option<Seen>,
    next_id: u32,
    /// No new curiosity before this brain time.
    ready_at: f32,
    pub(crate) best: Option<HighlightChoice>,
}

/// Same selection, extended or shrunk (rather than a new one elsewhere): they overlap by at
/// least half of the smaller one.
fn same_selection(a: RectI, b: RectI) -> bool {
    let i = a.intersect(&b);
    if i.is_empty() {
        return false;
    }
    let area = |r: RectI| r.width() as i64 * r.height() as i64;
    2 * area(i) >= area(a).min(area(b))
}

impl HighlightMemory {
    /// Fold this tick's selection in (`None`: nothing selected, or the feature is off).
    pub(crate) fn observe(&mut self, selection: Option<RectI>, dt: f32) {
        let sel = selection.filter(|r| r.width() >= MIN_SIDE && r.height() >= MIN_SIDE);
        self.seen = match (self.seen, sel) {
            (_, None) => None,
            (Some(s), Some(r)) if s.rect == r => Some(Seen { age: s.age + dt, still: s.still + dt, ..s }),
            (Some(s), Some(r)) if same_selection(s.rect, r) => Some(Seen { rect: r, age: s.age + dt, still: 0.0, ..s }),
            (_, Some(r)) => {
                self.next_id = self.next_id.wrapping_add(1);
                Some(Seen { id: self.next_id, rect: r, age: 0.0, still: 0.0, looked: false })
            }
        };
    }

    /// The selection's rectangle now, if it is still the one with this id.
    pub(crate) fn rect_of(&self, id: u32) -> Option<RectI> {
        self.seen.filter(|s| s.id == id).map(|s| s.rect)
    }

    /// Done with selection `id`: looked at it; leave selections alone for a while.
    pub(crate) fn looked(&mut self, id: u32, now: f32) {
        if let Some(s) = self.seen.as_mut().filter(|s| s.id == id) {
            s.looked = true;
        }
        self.ready_at = now + COOLDOWN_SECS;
    }

    /// The selection, if it is news worth a look (`going`: he is already on his way to it).
    fn news(&self, now: f32, going: Option<u32>) -> Option<Seen> {
        let s = self.seen.filter(|s| !s.looked)?;
        if Some(s.id) == going {
            return Some(s);
        }
        (now >= self.ready_at && s.still >= SETTLE_SECS && s.age <= STALE_SECS).then_some(s)
    }
}

impl Brain {
    /// The selection he is going to look at, if any.
    fn highlight_target(&self) -> Option<u32> {
        match &self.active {
            Active::Highlight(b) => Some(b.id),
            _ => None,
        }
    }

    /// Where to look at the current selection from: a vantage point a route really gets
    /// him to.
    pub(crate) fn score_highlight(&mut self, p: &Percepts, nav: &mut dyn Navigator) {
        let k = p.knight;
        let style = self.mind.travel_style();
        let going = self.highlight_target();
        let best = self.interest.highlight.news(self.now, going).and_then(|s| {
            let vantage = spots::vantage(p.terrain, s.rect, k.pos)?;
            if self.interest.is_blocked(PoiTarget::Spot(vantage.point)) {
                return None;
            }
            let here = vantage.point.distance(k.pos) < VANTAGE_TOL && k.is_grounded();
            let eta = if here {
                0.0
            } else {
                let r = nav.reach(k, NavGoal::Point(vantage.point), style)?;
                if !r.lands_on(vantage.point, VANTAGE_TOL) {
                    return None;
                }
                r.eta
            };
            Some(HighlightChoice { id: s.id, vantage, eta })
        });
        self.interest.highlight.best = best;
    }

    /// Utility of looking at highlighted text: curiosity, fading as the selection gets old;
    /// never while asleep / napping, away, fullscreen, or (to start) in mid-air.
    pub(crate) fn highlight_utility(&self, p: &Percepts) -> f32 {
        let s = &self.senses;
        let Some(c) = self.interest.highlight.best else { return 0.0 };
        let running = matches!(self.active, Active::Highlight(_));
        if s.fullscreen || s.user_away || self.asleep(p) || matches!(self.active, Active::Nap(_)) || (!running && !p.knight.is_grounded()) {
            return 0.0;
        }
        let age = self.interest.highlight.seen.map_or(0.0, |s| s.age);
        let fresh = 1.0 - 0.3 * (age / STALE_SECS).clamp(0.0, 1.0);
        let m = &self.mind;
        let want = 0.6 + 0.15 * m.personality.curiosity + 0.1 * m.needs.boredom;
        (want * fresh - 0.01 * c.eta).clamp(0.0, MAX_UTILITY)
    }
}

// --- the behaviour ------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Phase {
    Travel(Travel),
    /// At the vantage point: look at it (`looks` more acts to go).
    Look { beat: Beat, looks: u32 },
}

/// Go and look at the highlighted text.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LookAtHighlight {
    pub id: u32,
    rect: RectI,
    view: View,
    phase: Phase,
    /// Seconds the selection has been missing (cleared, or replaced by another one).
    missing: f32,
}

impl LookAtHighlight {
    pub fn start(ctx: &mut Ctx, choice: HighlightChoice) -> Self {
        let rect = ctx.interest.highlight.rect_of(choice.id).unwrap_or_default();
        let k = ctx.p.knight;
        ctx.mind.adjust(|n| &mut n.boredom, -0.05);
        // A curious knight takes a second look.
        let looks = if ctx.mind.personality.curiosity > 0.6 { 2 } else { 1 };
        let here = (choice.vantage.point.x - k.pos.x).abs() < VANTAGE_TOL && (choice.vantage.point.y - k.pos.y).abs() < 4.0 && k.is_grounded();
        let phase = if here {
            Phase::Look { beat: Beat::new(0.3), looks }
        } else {
            let style = style_for(ctx, choice.vantage.point);
            Phase::Travel(Travel::exact(ctx, choice.vantage.point, style, VANTAGE_TOL))
        };
        Self { id: choice.id, rect, view: choice.vantage.view, phase, missing: 0.0 }
    }

    pub fn tick(&mut self, ctx: &mut Ctx) -> Status {
        match ctx.interest.highlight.rect_of(self.id) {
            Some(r) => {
                self.rect = r;
                self.missing = 0.0;
            }
            None => {
                // Cleared (clicked elsewhere, typed over it) or a new selection: let it go.
                self.missing += ctx.dt();
                if self.missing > 0.4 {
                    ctx.nav.stop();
                    return Status::Done;
                }
            }
        }
        let center = self.rect.to_f().center();
        match &mut self.phase {
            Phase::Travel(t) => match t.tick(ctx) {
                TravelStep::Moving => Status::Running,
                TravelStep::Failed => Status::Failed,
                TravelStep::Arrived => {
                    ctx.face_toward(center.x);
                    let looks = if ctx.mind.personality.curiosity > 0.6 { 2 } else { 1 };
                    self.phase = Phase::Look { beat: Beat::new(0.25), looks };
                    Status::Running
                }
            },
            Phase::Look { beat, looks } => {
                ctx.nav.stop();
                let eye = ctx.p.knight.body().center();
                self.view = View::of(self.rect, center, eye);
                if !beat.busy() {
                    ctx.face_toward(center.x);
                }
                ctx.out.waiting = false; // eyes on it, even between looks
                if beat.tick(ctx, [0.5, 1.0]) {
                    if *looks == 0 {
                        ctx.interest.highlight.looked(self.id, ctx.now);
                        ctx.mind.adjust(|n| &mut n.boredom, -0.1);
                        return Status::Done;
                    }
                    *looks -= 1;
                    let act = ActWait::named(ctx, self.view.act());
                    beat.play(ctx, act, [0.5, 1.0]);
                }
                Status::Running
            }
        }
    }

    pub fn reason(&self, ctx: &Ctx) -> String {
        let c = self.rect.to_f().center();
        let what = match self.phase {
            Phase::Travel(t) => format!("going ({:?})", t.style),
            Phase::Look { beat, .. } => beat.doing_name(ctx).unwrap_or(self.view.act()).to_string(),
        };
        format!("highlight: what's that? text selected @ ({:.0},{:.0}) [{what}]", c.x, c.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extending_a_selection_keeps_it_new_ones_elsewhere_do_not() {
        let mut m = HighlightMemory::default();
        m.observe(Some(RectI::new(100, 100, 200, 120)), 0.1);
        let id = m.seen.unwrap().id;
        m.observe(Some(RectI::new(100, 100, 300, 120)), 0.1);
        assert_eq!(m.seen.unwrap().id, id, "extended");
        assert_eq!(m.seen.unwrap().still, 0.0);
        m.observe(Some(RectI::new(600, 500, 700, 520)), 0.1);
        assert_ne!(m.seen.unwrap().id, id, "a new selection");
        m.observe(None, 0.1);
        assert!(m.seen.is_none());
    }

    #[test]
    fn news_once_settled_until_looked_at_then_a_cooldown() {
        let mut m = HighlightMemory::default();
        let r = RectI::new(100, 100, 200, 120);
        m.observe(Some(r), 0.1);
        assert!(m.news(0.0, None).is_none(), "still dragging");
        for _ in 0..10 {
            m.observe(Some(r), 0.1);
        }
        let s = m.news(1.0, None).expect("settled");
        m.looked(s.id, 1.0);
        assert!(m.news(2.0, None).is_none(), "looked at");
        // Another selection during the cooldown: ignored; after it: news.
        let r2 = RectI::new(500, 300, 600, 320);
        for _ in 0..10 {
            m.observe(Some(r2), 0.1);
        }
        assert!(m.news(10.0, None).is_none());
        assert!(m.news(1.0 + COOLDOWN_SECS, None).is_some());
    }

    #[test]
    fn slivers_are_ignored() {
        let mut m = HighlightMemory::default();
        m.observe(Some(RectI::new(100, 100, 101, 120)), 0.1);
        assert!(m.seen.is_none());
    }
}
