//! The panel's Test tab: make the pet do one specific thing now (a visitor and the scene
//! after the talk, an enemy of one kind, a pastime, a spell...), optionally at a spot the
//! user clicks on screen ("📍 Place…"), and hear back what came of it.
//!
//! Part of the sandbox: the pet refuses every [`TestCommand`] until the user enabled it
//! (achievements paused until restart; see [`crate::Command::needs_sandbox`]).
use serde::{Deserialize, Serialize};

/// What follows a visitor's talk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scene {
    /// Whatever the rolls say (as a natural visit).
    Natural,
    /// Only the talk.
    TalkOnly,
    /// Sly's lesson.
    Lesson,
    /// Zote's spar.
    Spar,
    /// The calm scene: Cornifer's maps, Quirrel's view, Myla's song.
    Company,
    /// Hornet's race / duel.
    Race,
    Duel,
}

impl Scene {
    pub const ALL: [Scene; 7] = [Scene::Natural, Scene::TalkOnly, Scene::Lesson, Scene::Spar, Scene::Company, Scene::Race, Scene::Duel];
}

/// A visitor as the panel lists it (`Hello::visitors`, indexed like [`TestCommand::Visit`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VisitorEntry {
    /// Id in `npcs.json` ("cornifer").
    pub name: String,
    pub display: String,
    /// The scenes this visitor has besides the talk (and `Natural`): their button labels.
    pub scenes: Vec<(Scene, String)>,
}

/// Things that can go where the user clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Placeable {
    /// The training dummy.
    Dummy,
    /// The scarlet flame (the user then clicks it to summon Grimm).
    Flame,
    /// A Geo deposit (in the clicked window: the region follows its app).
    Deposit,
    Shiny,
    /// A placed bench of this style (`Hello::bench_styles`).
    Bench(u8),
    /// An enemy of this kind (`Hello::enemies`) dropping in.
    Enemy(u8),
    /// A Geo shower's portal.
    GeoShower,
    /// A visitor (`Hello::visitors`) arriving there, with the scene after the talk.
    Visitor { who: u8, scene: Scene },
    /// A shiny there, and the Dream Gate's cooldown cleared: pick a spot he can't walk to.
    DreamGate,
}

/// The Test tab's commands (`Command::Test`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestCommand {
    /// Visitor `who` (index into `Hello::visitors`) now, then `scene` after the talk.
    Visit { who: u8, scene: Scene },
    /// An enemy of kind `kind` (index into `Hello::enemies`) wanders in now.
    Enemy { kind: u8 },
    /// Taking in the view from a high window edge, now (needs a settled, reachable one).
    View,
    /// Stargazing now, whatever the hour (needs an open, reachable ledge).
    Stargaze,
    /// The Dream Gate's cooldown cleared: his next unreachable prize gets a gate at once.
    DreamGateReady,
    /// Unn's slug form (the crawl), now.
    Slug,
    /// Cycle every emote above his head (`--emote-test`), on / off.
    ToggleEmoteTest,
    /// Next left-click on screen places this (Esc / right-click cancels; times out).
    Place(Placeable),
    CancelPlace,
}

/// One line of feedback for the Test tab.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TestNote {
    /// Monotonic per pet process.
    pub seq: u64,
    /// Seconds since the pet started.
    pub t: f64,
    /// It went as asked (false: why it couldn't).
    pub ok: bool,
    pub text: String,
}

/// The Test tab's state (in `Status::test`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TestInfo {
    /// Waiting for a click to place this (its label).
    pub placing: Option<String>,
    /// Seconds left to click.
    pub placing_secs: f32,
    pub emote_test: bool,
    /// The latest notes, oldest first.
    pub notes: Vec<TestNote>,
}
