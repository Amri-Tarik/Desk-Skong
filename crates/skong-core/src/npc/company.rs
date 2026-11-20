//! Calm scenes: after the talk, a visitor may ask the knight to stay a while and simply
//! keep them company. Cornifer and the knight sit side by side with their maps open while
//! he hums; Quirrel and the knight sit on a high window edge taking in the view; Myla mines
//! beside a Geo deposit and sings while the knight listens.
//!
//! A scene is data (`company` in `npcs.json`): the invitation, the clip the visitor holds,
//! the knight act(s) for his part (`data/acts/company.json`), the quiet lines said now and
//! then, and the parting / abandon lines. The visitor runs it (`actor/company_run.rs`), the
//! brain's `Company` behaviour plays the knight's side, and the app decides whether a
//! scene fits the spot at all ([`CompanyNeeds`], checked by [`seat_beside`] /
//! [`high_up`] / [`beside_deposit`]).
use anyhow::{Result, bail};
use serde::Deserialize;

use crate::ai::rng::Rng;
use crate::anim::{ClipId, ClipLibrary};
use crate::geom::{RectF, Vec2};
use crate::portal::Footing;
use crate::terrain::{Ledge, LedgeKind, Terrain};

use super::Placement;

/// What a scene needs from the spot before it may start.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CompanyNeeds {
    /// Wherever the visit is (the knight sits if he can, else stands).
    #[default]
    Any,
    /// A window edge beside the visitor the knight can sit on.
    Seat,
    /// A seat high on its monitor: a view to take in.
    View,
    /// A Geo deposit right beside the visitor (arranged when they arrive).
    Deposit,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanyDef {
    /// Chance (0..1) the conversation ends in an invitation (when the spot fits).
    pub chance: f32,
    #[serde(default)]
    pub needs: CompanyNeeds,
    /// The invitation (one picked).
    pub invite: Vec<String>,
    /// The knight's part: act names from the acts files, `a|b` = the first that can start.
    pub knight: String,
    /// Clip the visitor holds while they sit together (default: idle).
    #[serde(default)]
    pub clip: Option<String>,
    /// Played once after the parting line (Quirrel turning back from the view).
    #[serde(default)]
    pub clip_end: Option<String>,
    /// Seconds together (random in range), counted while the knight is beside them.
    #[serde(default = "secs")]
    pub secs: [f32; 2],
    /// Quiet lines (hums, a remark) said now and then while together.
    #[serde(default)]
    pub lines: Vec<String>,
    #[serde(default = "every")]
    pub every: [f32; 2],
    /// The last word as they part.
    pub parting: Vec<String>,
    /// Said when it is cut short (the knight was called away).
    pub abandon: Vec<String>,
}

fn secs() -> [f32; 2] {
    [35.0, 55.0]
}
fn every() -> [f32; 2] {
    [8.0, 14.0]
}

/// [`CompanyDef`]'s clips resolved.
#[derive(Clone, Copy, Debug)]
pub struct CompanyClips {
    pub clip: ClipId,
    pub end: Option<ClipId>,
}

impl CompanyDef {
    pub fn validate(&self) -> Result<()> {
        let range_ok = |r: [f32; 2]| r[0].is_finite() && r[0] > 0.0 && r[1] >= r[0];
        if !(0.0..=1.0).contains(&self.chance) {
            bail!("company: chance must be within 0..1");
        }
        if !range_ok(self.secs) || !range_ok(self.every) {
            bail!("company: bad secs / every range");
        }
        if self.invite.is_empty() || self.parting.is_empty() || self.abandon.is_empty() {
            bail!("company: needs invite, parting and abandon lines");
        }
        if self.knight.split('|').any(|a| a.trim().is_empty()) {
            bail!("company: knight needs act names");
        }
        let all = self.invite.iter().chain(&self.lines).chain(&self.parting).chain(&self.abandon);
        if all.clone().any(|l| l.contains('{')) {
            bail!("company: lines are plain text (no placeholders)");
        }
        if all.clone().any(|l| l.trim().is_empty()) {
            bail!("company: empty line");
        }
        Ok(())
    }

    pub fn resolve(&self, lib: &ClipLibrary, idle: ClipId, who: &str) -> Result<CompanyClips> {
        let get = |n: &str| match lib.id(n) {
            Some(id) => Ok(id),
            None => bail!("npc '{who}': company clip '{n}' not in its library"),
        };
        Ok(CompanyClips { clip: self.clip.as_deref().map(get).transpose()?.unwrap_or(idle), end: self.clip_end.as_deref().map(get).transpose()? })
    }
}

/// How far the sitting knight's legs dangle below the seat (game units; as the app's seats).
const LEG_UNITS: f32 = 28.0 / 64.0;
/// Keep this far from a ledge's ends (px).
const MARGIN: f32 = 12.0;

/// The window edge under a visitor at `visitor` where the knight, at `knight_x`, can sit
/// beside them: not the desktop floor, spanning both, nothing within leg length under the
/// knight's seat.
pub fn seat_beside(terrain: &Terrain, visitor: Vec2, knight_x: f32, ppu: f32) -> Option<Ledge> {
    let legs = LEG_UNITS * ppu;
    let l = terrain
        .ledges
        .iter()
        .filter(|l| (l.y - visitor.y).abs() < 2.0 && l.x0 - 1.0 <= visitor.x && visitor.x <= l.x1 + 1.0)
        .max_by(|a, b| (a.x1 - a.x0).total_cmp(&(b.x1 - b.x0)))?;
    let spans = l.x0 + MARGIN <= knight_x && knight_x <= l.x1 - MARGIN;
    let blocked = terrain.ledges.iter().any(|o| o.x0 <= knight_x && knight_x <= o.x1 && o.y > l.y + 0.5 && o.y < l.y + legs);
    (l.kind != LedgeKind::Ground && spans && !blocked).then_some(*l)
}

/// High on its monitor (upper half of the work area): somewhere with a view.
pub fn high_up(y: f32, work: RectF) -> bool {
    y <= work.top + 0.5 * work.height()
}

/// Where a visitor stands beside a Geo deposit: on the deposit's footing, on the side away
/// from where the knight mines it (`at`), else further along past him. `deposits` are
/// (rock bottom center, the knight's mining spot); `ok` checks room and reachability.
pub fn beside_deposit(deposits: &[(Vec2, Vec2)], footings: &[Footing], ppu: f32, rng: &mut Rng, ok: &mut dyn FnMut(Vec2) -> bool) -> Option<(Placement, Vec2)> {
    if deposits.is_empty() {
        return None;
    }
    let start = rng.next_u32() as usize;
    for k in 0..deposits.len() {
        let (center, at) = deposits[(start + k) % deposits.len()];
        let Some(f) = footings.iter().find(|f| (f.y - center.y).abs() < 2.0 && f.x0 <= center.x && center.x <= f.x1) else { continue };
        let reach = (center.x - at.x).abs().max(1.0 * ppu);
        let away = if at.x < center.x { 1.0 } else { -1.0 };
        let xs = [1.0, 1.3, 1.6].map(|m| center.x + away * reach * m).into_iter().chain([at.x - away * 1.8 * ppu, at.x - away * 2.4 * ppu]);
        for x in xs {
            let p = Vec2::new(x, f.y);
            if f.x0 + MARGIN <= x && x <= f.x1 - MARGIN && ok(p) {
                return Some((Placement { feet: p, owner: f.owner, walk_to: None }, center));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;
