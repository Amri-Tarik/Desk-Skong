//! The control panel's Test tab ([`TestCommand`]): make the pet do one specific thing now —
//! a visitor and the scene after the talk, an enemy of one kind, the view, the stars, the
//! slug, the emote cycle — or put something where the user clicks ("📍 Place…": the next
//! left click anywhere is captured by `skong_win::click_capture`, swallowed, and snapped to
//! the nearest ledge at or below it, never on a game's monitor; Esc / right-click cancels;
//! it times out). What came of each request goes back to the panel as a [`TestNote`].
//!
//! Placed props skip the "lasting ledge" age rule for *where* they may go; they still only
//! show on a ledge that has settled (a window moved in the last ~15 s shows them once it
//! has). The precursor of the sandbox (later behind an "achievements off" gate).
use std::collections::VecDeque;
use std::time::Instant;

use skong_core::ai::PastimeKind;
use skong_core::desktop::WindowId;
use skong_core::geom::Vec2;
use skong_core::npc::Placement;
use skong_core::npc::forced::ForcedScene;
use skong_core::placement::snap_below;
use skong_core::terrain::LedgeKind;
use skong_ipc::{Placeable, Scene, StakesCommand, TestCommand, TestInfo, TestNote, VisitorEntry};
use skong_win::click_capture::{Captured, ClickCapture};
use skong_win::monitor;

use super::Pet;
use crate::placehint::PlaceHint;

/// Seconds to click a spot before placing gives up.
const PLACE_SECS: f32 = 30.0;
/// Notes kept for the panel.
const NOTES: usize = 8;

struct Placing {
    what: Placeable,
    label: String,
    capture: ClickCapture,
    hint: Option<PlaceHint>,
    left: f32,
}

pub(super) struct TestDesk {
    started: Instant,
    seq: u64,
    notes: VecDeque<TestNote>,
    placing: Option<Placing>,
    /// Where the next scarlet flame goes (a spot the user picked).
    pub lantern_at: Option<(Vec2, Option<WindowId>)>,
    /// Last "the troupe waits for…" reason passed on.
    grimm_held: Option<&'static str>,
}

impl TestDesk {
    pub fn new() -> Self {
        Self { started: Instant::now(), seq: 0, notes: VecDeque::new(), placing: None, lantern_at: None, grimm_held: None }
    }
}

/// The scene the panel asked for, as the visitors know it (`Natural`: none forced).
fn forced(scene: Scene) -> Option<ForcedScene> {
    match scene {
        Scene::Natural => None,
        Scene::TalkOnly => Some(ForcedScene::TalkOnly),
        Scene::Lesson => Some(ForcedScene::Lesson),
        Scene::Spar => Some(ForcedScene::Spar),
        Scene::Company => Some(ForcedScene::Company),
        Scene::Race => Some(ForcedScene::Race),
        Scene::Duel => Some(ForcedScene::Duel),
    }
}

fn scene_of(s: ForcedScene) -> Scene {
    match s {
        ForcedScene::TalkOnly => Scene::TalkOnly,
        ForcedScene::Lesson => Scene::Lesson,
        ForcedScene::Spar => Scene::Spar,
        ForcedScene::Company => Scene::Company,
        ForcedScene::Race => Scene::Race,
        ForcedScene::Duel => Scene::Duel,
    }
}

/// The button label for visitor `name`'s scene.
fn scene_label(name: &str, s: ForcedScene) -> String {
    match (name, s) {
        ("sly", ForcedScene::Lesson) => "Sly lesson".into(),
        ("zote", ForcedScene::Spar) => "Zote spar".into(),
        ("cornifer", ForcedScene::Company) => "Cornifer maps".into(),
        ("quirrel", ForcedScene::Company) => "Quirrel view".into(),
        ("myla", ForcedScene::Company) => "Myla song".into(),
        ("hornet", ForcedScene::Race) => "Hornet race".into(),
        ("hornet", ForcedScene::Duel) => "Hornet duel".into(),
        (_, s) => {
            let l = s.label().trim_start_matches("a ");
            let mut c = l.chars();
            c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
        }
    }
}

impl Pet {
    /// The visitors as the panel lists them (Hello).
    pub(super) fn test_visitors(&self) -> Vec<VisitorEntry> {
        let Some(v) = &self.visitors else { return Vec::new() };
        v.roster().into_iter().map(|(name, display, scenes)| VisitorEntry { scenes: scenes.into_iter().map(|s| (scene_of(s), scene_label(&name, s))).collect(), name, display }).collect()
    }

    /// One line for the panel (and the log).
    pub(super) fn note(&mut self, ok: bool, text: impl Into<String>) {
        let text = text.into();
        if ok {
            log::info!("test: {text}");
        } else {
            log::warn!("test: {text}");
        }
        let t = &mut self.test;
        t.seq += 1;
        t.notes.push_back(TestNote { seq: t.seq, t: t.started.elapsed().as_secs_f64(), ok, text });
        while t.notes.len() > NOTES {
            t.notes.pop_front();
        }
    }

    pub(super) fn test_info(&self) -> TestInfo {
        TestInfo {
            placing: self.test.placing.as_ref().map(|p| p.label.clone()),
            placing_secs: self.test.placing.as_ref().map_or(0.0, |p| p.left),
            emote_test: self.emote_test,
            notes: self.test.notes.iter().cloned().collect(),
        }
    }

    pub(super) fn apply_test(&mut self, c: TestCommand) {
        match c {
            TestCommand::Visit { who, scene } => self.test_visit(who, scene, None),
            TestCommand::Enemy { kind } => self.test_enemy(kind, None),
            TestCommand::View => {
                self.brain.force_pastime(PastimeKind::View);
                self.note(true, "taking in the view: asked (he looks for a settled, high window edge he can reach)");
            }
            TestCommand::Stargaze => {
                self.brain.force_pastime(PastimeKind::Stargaze);
                self.note(true, "stargazing: asked (any hour; he looks for an open ledge he can reach)");
            }
            TestCommand::DreamGateReady => {
                self.brain.ready_dream_gate();
                self.note(true, "dream gate ready: the next prize no route reaches gets a gate at once");
            }
            TestCommand::Slug => match self.data.acts.id("slug_crawl") {
                Some(id) if self.ch.is_grounded() => {
                    self.seat_y = None;
                    self.start_act(id);
                    self.note(true, "slug form: crawling");
                }
                Some(_) => self.note(false, "slug form: he has to be standing on something"),
                None => self.note(false, "slug form: no slug_crawl act in the data"),
            },
            TestCommand::ToggleEmoteTest => {
                self.emote_test = !self.emote_test;
                if !self.emote_test {
                    self.emotes.hide();
                }
                self.note(true, format!("emote cycle {}", if self.emote_test { "on (every 2.5 s)" } else { "off" }));
            }
            TestCommand::Place(what) => self.begin_place(what),
            TestCommand::CancelPlace => {
                if let Some(p) = self.test.placing.take() {
                    self.note(false, format!("placing {}: cancelled", p.label));
                }
            }
        }
    }

    /// Visitor `who` (Hello index) now, maybe at a picked spot, then `scene`.
    fn test_visit(&mut self, who: u8, scene: Scene, at: Option<Placement>) {
        let Some(v) = self.visitors.as_mut() else {
            self.note(false, "visitors aren't loaded");
            return;
        };
        let Some((name, display, _)) = v.roster().into_iter().nth(who as usize) else {
            self.note(false, format!("no visitor #{who}"));
            return;
        };
        let res = v.invite_scene(&name, forced(scene), at);
        match res {
            Ok(()) => {
                let what = match scene {
                    Scene::Natural => String::new(),
                    s => format!(", then {}", forced(s).map_or("", |f| f.label())),
                };
                self.note(true, format!("{display} invited{what} (arrives once the nav graph is ready)"));
            }
            Err(e) => self.note(false, e),
        }
    }

    fn test_enemy(&mut self, kind: u8, at: Option<(Vec2, Option<WindowId>)>) {
        let res = match self.enemies.as_mut() {
            Some(e) => e.send_kind(kind as usize, at),
            None => Err("enemies aren't loaded".into()),
        };
        if let Err(e) = res {
            self.note(false, e);
        }
    }

    fn placeable_label(&self, what: Placeable) -> String {
        match what {
            Placeable::Dummy => "training dummy".into(),
            Placeable::Flame => "scarlet flame".into(),
            Placeable::Deposit => "Geo deposit".into(),
            Placeable::Shiny => "shiny".into(),
            Placeable::Bench(s) => format!("{} bench", self.benches.styles.get(s as usize).map_or("?", |b| b.name.as_str())),
            Placeable::Enemy(k) => self.enemies.as_ref().and_then(|e| e.displays().get(k as usize).cloned()).unwrap_or_else(|| "enemy".into()),
            Placeable::GeoShower => "Geo shower".into(),
            Placeable::Visitor { who, .. } => self.visitors.as_ref().and_then(|v| v.roster().into_iter().nth(who as usize)).map_or_else(|| "visitor".into(), |r| r.1),
            Placeable::DreamGate => "Dream Gate target".into(),
        }
    }

    fn begin_place(&mut self, what: Placeable) {
        let label = self.placeable_label(what);
        // A new placing replaces one under way (its hooks go first).
        self.test.placing = None;
        match ClickCapture::start() {
            Ok(capture) => {
                let hint = PlaceHint::new(self.fx.capturable || self.settings.share_visible).map_err(|e| log::warn!("place hint: {e:#}")).ok();
                self.test.placing = Some(Placing { what, label: label.clone(), capture, hint, left: PLACE_SECS });
                self.note(true, format!("placing {label}: click a spot (Esc / right-click cancels)"));
            }
            Err(e) => self.note(false, format!("placing {label}: {e:#}")),
        }
    }

    /// Every loop turn (paused too): the click being placed, the cue by the cursor, the
    /// timeout, and the notes from the visitors, enemies, troupe and brain.
    pub(super) fn test_tick(&mut self, dt: f32) {
        let mut done = None;
        if let Some(p) = self.test.placing.as_mut() {
            p.left -= dt;
            match p.capture.poll() {
                Some(c) => done = Some((p.what, p.label.clone(), Some(c))),
                None if p.left <= 0.0 => done = Some((p.what, p.label.clone(), None)),
                None => {
                    if let (Some(h), Some((x, y))) = (p.hint.as_mut(), monitor::cursor_pos()) {
                        let text = format!("Place {}: click a spot  ·  Esc / right-click cancels  ·  {:.0} s", p.label, p.left.ceil());
                        h.update(&text, x, y);
                    }
                }
            }
        }
        if let Some((what, label, got)) = done {
            self.test.placing = None; // unhooks
            match got {
                Some(Captured::At(x, y)) => self.place(what, &label, Vec2::new(x as f32, y as f32)),
                Some(Captured::Cancelled) => self.note(false, format!("placing {label}: cancelled")),
                None => self.note(false, format!("placing {label}: no click within {PLACE_SECS:.0} s")),
            }
        }
        let mut lines: Vec<(bool, String)> = Vec::new();
        if let Some(v) = self.visitors.as_mut() {
            lines.append(&mut v.notes);
        }
        if let Some(e) = self.enemies.as_mut() {
            lines.append(&mut e.notes);
        }
        lines.extend(self.brain.take_test_notes().into_iter().map(|n| (!n.starts_with("can't") && !n.contains("didn't"), n)));
        let held = self.troupe.as_ref().and_then(|t| t.held_by());
        if held != self.test.grimm_held {
            self.test.grimm_held = held;
            if let Some(why) = held {
                lines.push((false, format!("the troupe master waits for a good moment: {why}")));
            }
        }
        for (ok, text) in lines {
            // Already logged where they came from.
            let t = &mut self.test;
            t.seq += 1;
            t.notes.push_back(TestNote { seq: t.seq, t: t.started.elapsed().as_secs_f64(), ok, text });
            while t.notes.len() > NOTES {
                t.notes.pop_front();
            }
        }
    }

    /// The user clicked `click` to place `what`.
    fn place(&mut self, what: Placeable, label: &str, click: Vec2) {
        let scale = self.settings.scale;
        let half = match what {
            Placeable::Dummy => self.pastimes.dummy_half_width(),
            Placeable::Deposit => self.discoveries.deposit_half_width(),
            Placeable::Shiny | Placeable::DreamGate => self.shinies.half_width(),
            Placeable::Bench(s) => self.benches.art(s as usize).map_or(60.0 * scale, |i| 0.5 * i.width as f32),
            _ => 0.6 * 64.0 * scale,
        };
        let desk = self.watcher.snapshot();
        let s = match snap_below(&self.terrain, &desk.monitors, &self.games.game_monitors, click, half) {
            Ok(s) => s,
            Err(e) => return self.note(false, format!("can't place the {label} at ({:.0},{:.0}): {e}", click.x, click.y)),
        };
        let on = match s.owner.and_then(|id| desk.window(id)) {
            Some(w) => format!("{}'s {}", w.process, if s.kind == LedgeKind::Content { "content line" } else { "title bar" }),
            None => "the floor".to_string(),
        };
        let settled = self.discoveries.settled(&self.terrain, s.owner, s.at.y, s.span.0, s.span.1);
        let wait = if settled { "" } else { " (that edge moved lately: it shows once it has settled, ≤ 15 s)" };
        let at = format!("({:.0},{:.0}) on {on}", s.at.x, s.at.y);
        match what {
            Placeable::Dummy => {
                if self.pastimes.place_dummy_at(&self.terrain, s.at, s.owner) {
                    // The brain's own test, so the panel says whether he will practise there.
                    if self.dummy_practisable_at(s.at, s.span) {
                        self.note(true, format!("placed the training dummy at {at}{wait}; he'll go and practise"));
                    } else {
                        self.note(false, format!("placed the training dummy at {at}{wait}, but he can't practise there: no stand spot beside it that his routes (a stroll or a run) land on"));
                    }
                } else {
                    self.note(false, "training dummy: that window is gone");
                }
            }
            Placeable::Shiny | Placeable::DreamGate => {
                if !self.shinies.place_at(&self.terrain, s.at, s.owner) {
                    return self.note(false, format!("{label}: that window is gone"));
                }
                if what == Placeable::DreamGate {
                    self.brain.ready_dream_gate();
                    self.note(true, format!("a shiny at {at}{wait}, the Dream Gate ready: if no route gets him there, he gates to it"));
                } else {
                    self.note(true, format!("placed a shiny at {at}{wait}"));
                }
            }
            Placeable::Deposit => {
                let Some(id) = s.owner else {
                    return self.note(false, "Geo deposits grow in windows: click on a window's title bar or a line in it");
                };
                let desk = self.watcher.snapshot();
                match self.discoveries.force_deposit_at(id, s.at, (desk, &self.terrain), &self.benches, &self.occupancy) {
                    Ok(region) => self.note(true, format!("a Geo deposit ({region} rock: the region follows the app) at {at}{wait}")),
                    Err(e) => self.note(false, format!("no Geo deposit there: {e}")),
                }
            }
            Placeable::Bench(style) => match self.benches.place(style as usize, s.at - Vec2::new(0.0, 1.0), &self.terrain, &self.occupancy) {
                Ok(()) => {
                    self.win.raise();
                    self.note(true, format!("placed the {label} at {at}"));
                }
                Err(e) => self.note(false, format!("{label}: {e:#}")),
            },
            // The enemies say how it went (next tick).
            Placeable::Enemy(kind) => self.test_enemy(kind, Some((s.at, s.owner))),
            Placeable::GeoShower => self.shower_at(s.at, &at),
            Placeable::Flame => match self.troupe.as_mut() {
                Some(t) => {
                    self.test.lantern_at = Some((s.at, s.owner));
                    t.request(true);
                    self.note(true, format!("a scarlet flame at {at} at the next good moment: click it to summon him"));
                }
                None => self.note(false, "the troupe isn't loaded (data/grimm.json)"),
            },
            Placeable::Visitor { who, scene } => self.test_visit(who, scene, Some(Placement { feet: s.at, owner: s.owner, walk_to: None })),
        }
    }

    /// A Geo shower's portal right above `feet`.
    fn shower_at(&mut self, feet: Vec2, at: &str) {
        match self.open_shower_at(feet) {
            Some(amount) => self.note(true, format!("a Geo shower ({amount} geo) through a portal at {at}")),
            None => self.note(false, "Geo shower: the portals come with the visitors, which aren't loaded"),
        }
    }

    /// Test panel: the stakes buttons say what they did.
    pub(super) fn note_stakes(&mut self, c: StakesCommand) {
        match c {
            StakesCommand::Die => self.note(true, "the knight dies (a Shade where he fell, then he respawns)"),
            StakesCommand::HuntShade if !self.stakes.has_shade() => self.note(false, "no Shade to hunt (die first)"),
            StakesCommand::HuntShade => self.note(true, "after his Shade now"),
            StakesCommand::Cast(i) if !self.ch.is_grounded() => self.note(false, format!("{}: he has to be standing", skong_ipc::SPELLS.get(i as usize).map_or("spell", |s| s.1))),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_scenes_map_onto_forced_scenes_and_back() {
        assert_eq!(forced(Scene::Natural), None);
        for s in Scene::ALL.into_iter().filter(|s| *s != Scene::Natural) {
            assert_eq!(forced(s).map(scene_of), Some(s));
        }
        assert_eq!(scene_label("myla", ForcedScene::Company), "Myla song");
        assert_eq!(scene_label("someone", ForcedScene::Company), "Calm scene");
        assert_eq!(scene_label("someone", ForcedScene::Spar), "Spar");
    }
}
