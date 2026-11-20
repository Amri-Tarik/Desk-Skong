//! Desk-Skong control protocol: the pet (`skong.exe`) serves a named pipe that control
//! panels (`skong-panel.exe`) connect to.
//!
//! * Transport: `\\.\pipe\desk-skong`, byte mode, local clients only ([`pipe`]).
//! * Framing: one JSON document per line (`serde_json` never emits raw newlines).
//! * Panel → pet: [`PanelMsg`]. Pet → panel: [`PetMsg`] — a [`Hello`] on connect, the log
//!   backlog, then [`Status`] ~4 Hz and new [`LogLine`]s as they are produced, plus
//!   [`AchievementsInfo`] on connect and whenever an achievement unlocks.
//!
//! Any number of panels may connect; the pet keeps running when one goes away.
use serde::{Deserialize, Serialize};

pub mod achievements;
pub mod pipe;
pub mod stakes;

pub use achievements::{AchievementEntry, AchievementsInfo, UnlockedAchievement};
pub use pipe::{ClientId, Connection, PIPE_NAME, Server, ServerEvent, connect};
pub use stakes::{SPELLS, StakesCommand, StakesInfo};

/// Everything the user can ask the pet for, from hotkeys, the tray menu or a panel.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    ToggleManual,
    /// Explicit manual mode (panels; "drive while focused").
    SetManual(bool),
    RandomAct,
    Wake,
    RespawnAtCursor,
    NextSkin,
    SetSkin(usize),
    SetScale(f32),
    SetBother(f32),
    /// Show the pet in screen shares and screenshots (Discord, OBS...); off by default.
    ToggleShareVisible,
    /// Fight the cursor, round after round (until toggled off).
    ToggleFightMode,
    /// Curious about highlighted text (UI Automation, rectangles only); on by default.
    ToggleTextCuriosity,
    PlaceBenchAtCursor(usize),
    PlaceBenchNearKnight(usize),
    ClearBenches,
    /// Reveal a find in the window under the knight: 0 bench, 1 geo deposit.
    Discover(u8),
    /// Forget everything found in windows.
    ForgetDiscoveries,
    TogglePause,
    /// Live content terrain (screen capture) on/off.
    ToggleVision,
    /// Full-screen terrain / nav-graph overlay on/off.
    ToggleDebugOverlay,
    /// Which layers the debug overlay draws.
    SetOverlayLayers(OverlayLayers),
    /// Launch `skong-panel.exe` next to the pet executable.
    OpenPanel,
    /// Bring an NPC visitor now (debug; visits also happen on their own).
    InviteVisitor,
    /// Let an enemy wander in (debug / for fun).
    SendEnemy,
    /// Start a gauntlet (waves of enemies, a reward) at the next good moment.
    StartGauntlet,
    /// Debug (the panel's hidden Debug section only): the secret boss at the next good
    /// moment; `lantern`: light his scarlet flame first instead of skipping to the show.
    SummonGrimm { lantern: bool },
    /// The knight dream-nails the window he stands on (thought bubble).
    DreamNail,
    /// Path-find to the cursor (Stroll, or Hurry with the full movement kit).
    GoToCursor { hurry: bool },
    /// Death / Shade / Geo / spell triggers.
    Stakes(StakesCommand),
    /// Forget every unlocked achievement and all progress (debug).
    ResetAchievements,
    Quit,
}

/// Debug overlay layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayLayers {
    /// Ledges and walls.
    pub terrain: bool,
    /// Nav surfaces and every edge.
    pub nav: bool,
    /// The current route (drawn even when `nav` is off).
    pub route: bool,
}

impl Default for OverlayLayers {
    fn default() -> Self {
        Self { terrain: true, nav: true, route: true }
    }
}

/// Keys held in a focused panel while it drives the knight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ManualKeys {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub jump: bool,
    pub dash: bool,
    pub superdash: bool,
    pub walk: bool,
    pub sprint: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<log::Level> for LogLevel {
    fn from(l: log::Level) -> Self {
        match l {
            log::Level::Error => LogLevel::Error,
            log::Level::Warn => LogLevel::Warn,
            log::Level::Info => LogLevel::Info,
            log::Level::Debug => LogLevel::Debug,
            log::Level::Trace => LogLevel::Trace,
        }
    }
}

impl From<LogLevel> for log::Level {
    fn from(l: LogLevel) -> Self {
        match l {
            LogLevel::Error => log::Level::Error,
            LogLevel::Warn => log::Level::Warn,
            LogLevel::Info => log::Level::Info,
            LogLevel::Debug => log::Level::Debug,
            LogLevel::Trace => log::Level::Trace,
        }
    }
}

impl LogLevel {
    pub const ALL: [LogLevel; 5] = [LogLevel::Error, LogLevel::Warn, LogLevel::Info, LogLevel::Debug, LogLevel::Trace];

    pub fn name(self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
            LogLevel::Trace => "TRACE",
        }
    }
}

/// Panel → pet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PanelMsg {
    Command(Command),
    /// Sent whenever the held keys or the panel's "driving" state change. `active: false`
    /// hands control back (focus lost, driving disabled).
    ManualInput { active: bool, keys: ManualKeys },
    /// Most verbose level captured for `skong*` targets (stderr keeps `RUST_LOG`).
    SetLogLevel(LogLevel),
}

/// Pet → panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PetMsg {
    Hello(Hello),
    Status(Box<Status>),
    Logs(Vec<LogLine>),
    /// Hidden achievements: on connect and whenever one unlocks (not in the status).
    Achievements(AchievementsInfo),
}

/// Static information, sent once per connection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hello {
    pub version: String,
    pub pid: u32,
    /// Display names, indexed like [`Command::SetSkin`].
    pub skins: Vec<String>,
    /// Indexed like [`Command::PlaceBenchNearKnight`].
    pub bench_styles: Vec<String>,
    pub scales: Vec<(String, f32)>,
    pub bother_levels: Vec<(String, f32)>,
    pub log_level: Option<LogLevel>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub paused: bool,
    pub manual: bool,
    /// A panel is currently driving the knight.
    pub driven: bool,
    /// Fighting the cursor on request (round after round).
    pub fight_mode: bool,
    /// Visible in screen shares and screenshots.
    pub share_visible: bool,
    /// Curious about highlighted text (on by default).
    pub text_curiosity: bool,
    pub vision: bool,
    pub overlay: bool,
    pub overlay_layers: OverlayLayers,
    pub skin: usize,
    pub scale: f32,
    pub bother: f32,
    pub benches_placed: usize,
    /// Per bench style: can another one be placed?
    pub benches_available: Vec<bool>,
    pub knight: KnightInfo,
    pub terrain: TerrainInfo,
    pub capture: Option<CaptureInfo>,
    pub process: ProcessInfo,
    /// One-line summary of what the brain is doing.
    pub brain: Option<String>,
    pub nav: Option<NavInfo>,
    /// Wallet, Shade, soul (absent from older pets).
    pub stakes: Option<StakesInfo>,
    /// Connected panels.
    pub panels: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KnightInfo {
    pub x: f32,
    pub y: f32,
    pub state: String,
    pub clip: String,
    pub act: Option<String>,
    pub facing_right: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerrainInfo {
    pub ground: usize,
    pub one_way: usize,
    pub content: usize,
    pub walls: usize,
    pub windows: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureInfo {
    pub running: bool,
    /// Content updates consumed since start.
    pub updates: u64,
    pub frames: u64,
    pub last_analysis_ms: f32,
    pub last_latency_ms: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessInfo {
    /// Since the previous status, in % of one core.
    pub cpu_percent: f32,
    pub working_set_mb: f32,
}

/// Navigation graph + current motion, filled by the app when nav is wired in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavInfo {
    pub generation: u64,
    pub surfaces: usize,
    /// `(move kind name, count)` in `MoveKind::ALL` order.
    pub edges: Vec<(String, usize)>,
    pub build_ms: f32,
    pub pairs: usize,
    pub cache_hits: usize,
    pub sims: usize,
    pub motion: Option<MotionInfo>,
}

/// What the brain's navigator is doing right now.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MotionInfo {
    pub goal: String,
    pub style: String,
    /// Move kinds of the current route, in order.
    pub moves: Vec<String>,
    pub eta: f32,
    /// Follower status (Moving / Arrived / Failed / …).
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogLine {
    /// Monotonic per pet process; panels use it to drop duplicates.
    pub seq: u64,
    /// Seconds since the pet started.
    pub t: f64,
    pub level: LogLevel,
    pub target: String,
    pub msg: String,
}

/// Encode one message as a protocol line (JSON + `\n`).
pub fn encode<T: Serialize>(msg: &T) -> String {
    // These plain data types always serialize (no maps with non-string keys).
    let mut s = serde_json::to_string(msg).unwrap_or_default();
    s.push('\n');
    s
}

/// Decode one protocol line (surrounding whitespace allowed).
pub fn decode<'a, T: Deserialize<'a>>(line: &'a str) -> serde_json::Result<T> {
    serde_json::from_str(line.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: Serialize + for<'a> Deserialize<'a> + PartialEq + std::fmt::Debug>(v: T) {
        let line = encode(&v);
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1, "one line: {line:?}");
        assert_eq!(decode::<T>(&line).unwrap(), v);
    }

    #[test]
    fn commands_round_trip() {
        for c in [
            Command::ToggleManual,
            Command::SetManual(true),
            Command::SetSkin(3),
            Command::SetScale(0.6),
            Command::SetBother(0.25),
            Command::PlaceBenchNearKnight(1),
            Command::SetOverlayLayers(OverlayLayers { terrain: false, nav: true, route: true }),
            Command::ToggleDebugOverlay,
            Command::StartGauntlet,
            Command::SummonGrimm { lantern: true },
            Command::ResetAchievements,
            Command::Quit,
        ] {
            round_trip(PanelMsg::Command(c));
        }
        round_trip(PanelMsg::ManualInput { active: true, keys: ManualKeys { left: true, jump: true, ..Default::default() } });
        round_trip(PanelMsg::SetLogLevel(LogLevel::Debug));
    }

    #[test]
    fn pet_messages_round_trip() {
        round_trip(PetMsg::Hello(Hello { version: "0.2.0".into(), pid: 42, skins: vec!["Vanilla".into(), "Sly\nnewline".into()], scales: vec![("Tiny".into(), 0.4)], ..Default::default() }));
        let mut s = Status { paused: true, brain: Some("napping".into()), ..Default::default() };
        s.knight.act = Some("sit".into());
        s.capture = Some(CaptureInfo { running: true, updates: 5, frames: 9, last_analysis_ms: 1.5, last_latency_ms: 20.0 });
        s.nav = Some(NavInfo { surfaces: 4, edges: vec![("Walk".into(), 3)], motion: Some(MotionInfo { goal: "window".into(), moves: vec!["Jump".into()], ..Default::default() }), ..Default::default() });
        round_trip(PetMsg::Status(Box::new(s)));
        round_trip(PetMsg::Logs(vec![LogLine { seq: 1, t: 0.5, level: LogLevel::Warn, target: "skong::app".into(), msg: "multi\nline \"quoted\"".into() }]));
    }

    #[test]
    fn achievements_round_trip_and_hide_locked_ones() {
        let unlocked = UnlockedAchievement { id: "pogo".into(), title: "Bounce".into(), description: "sprang\noff \"it\"".into(), at: 1_763_500_000 };
        let info = AchievementsInfo {
            entries: vec![
                AchievementEntry { hint: Some("the nail".into()), unlocked: Some(unlocked) },
                AchievementEntry { hint: Some("a visitor".into()), unlocked: None },
                AchievementEntry { hint: None, unlocked: None },
            ],
        };
        assert_eq!((info.unlocked(), info.total()), (1, 3));
        round_trip(PetMsg::Achievements(info.clone()));
        // A locked entry is a hint and nothing else on the wire.
        let line = encode(&PetMsg::Achievements(info));
        assert!(line.contains(r#"{"hint":"a visitor","unlocked":null}"#), "{line}");
        assert_eq!(encode(&PanelMsg::Command(Command::ResetAchievements)), "{\"Command\":\"ResetAchievements\"}\n");
        // Missing fields default.
        let e: AchievementsInfo = decode("{}").unwrap();
        assert_eq!(e.total(), 0);
    }

    #[test]
    fn stable_wire_format() {
        // Panels and pets of neighbouring versions should still understand each other.
        assert_eq!(encode(&PanelMsg::Command(Command::TogglePause)), "{\"Command\":\"TogglePause\"}\n");
        assert_eq!(encode(&PanelMsg::Command(Command::SetSkin(2))), "{\"Command\":{\"SetSkin\":2}}\n");
        // Missing fields default (e.g. an older pet without `nav`).
        let s: Status = decode("{\"paused\":true}").unwrap();
        assert!(s.paused && s.nav.is_none());
        assert!(decode::<PanelMsg>("{\"Command\":\"NoSuchThing\"}").is_err());
    }
}
