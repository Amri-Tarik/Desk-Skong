//! The Test tab: make the pet do one specific thing now, section by section (visitors and
//! their scenes, enemies, Grimm, pastimes and props, discoveries and Geo, the knight), with
//! "📍" buttons that put the thing where you click next on screen, and the pet's feedback
//! (what came of each request, why one couldn't run) at the top.
//!
//! The precursor of the sandbox: it will later sit behind an "achievements off for this
//! session" gate (not yet: achievements still count).
use eframe::egui::{self, Color32, RichText};
use skong_ipc::{Command, Hello, Placeable, SPELLS, Scene, StakesCommand, Status, TestCommand, TestInfo};

use super::Panel;

/// The "place where I click" companion of a button.
const PIN: &str = "📍";
/// Discover kinds (`Command::Discover`), as the pet numbers them.
const DISCOVER_BENCH: u8 = 0;
const DISCOVER_DEPOSIT: u8 = 1;
const DISCOVER_SHINY: u8 = 2;
const DISCOVER_DUMMY: u8 = 3;
const DISCOVER_LUMAFLY: u8 = 4;

const PIN_TIP: &str = "📍 Place: your next left click anywhere on screen picks the spot (the click doesn't reach the app under it); it lands on the nearest ledge at or below the click — a title bar, a content line, the floor — never on a game's monitor. Esc or right-click cancels; 30 s to click. Placed props skip the 'lasting ledge' rule, but on a window moved in the last ~15 s they show once it has settled.";

fn test(c: TestCommand) -> Command {
    Command::Test(c)
}

fn place(p: Placeable) -> Command {
    Command::Test(TestCommand::Place(p))
}

/// A button, its hover hint, and optionally a 📍 companion placing `pin`.
fn row(ui: &mut egui::Ui, out: &mut Vec<Command>, label: &str, tip: &str, now: Option<Command>, pin: Option<Placeable>) {
    if let Some(c) = now
        && ui.button(label).on_hover_text(tip).clicked()
    {
        out.push(c);
    }
    if let Some(p) = pin {
        let text = if now.is_some() { PIN.to_string() } else { format!("{PIN} {label}") };
        if ui.button(text).on_hover_text(format!("{tip}\n\n{PIN_TIP}")).clicked() {
            out.push(place(p));
        }
    }
    ui.add_space(6.0);
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(true).show(ui, body);
}

/// Placing in progress and the latest notes, newest first.
fn feedback(ui: &mut egui::Ui, out: &mut Vec<Command>, info: Option<&TestInfo>) {
    let Some(info) = info else {
        ui.weak("this pet sends no test feedback (older version)");
        return;
    };
    if let Some(what) = &info.placing {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{PIN} Placing {what}: click a spot on screen ({:.0} s)", info.placing_secs.ceil())).strong().color(Color32::from_rgb(235, 150, 90)));
            if ui.button("Cancel").on_hover_text("or press Esc / right-click anywhere").clicked() {
                out.push(test(TestCommand::CancelPlace));
            }
        });
    }
    if info.notes.is_empty() {
        ui.weak("results show here");
    }
    for n in info.notes.iter().rev().take(6) {
        let (mark, color) = if n.ok { ("✔", Color32::from_rgb(110, 200, 120)) } else { ("✖", Color32::from_rgb(235, 160, 70)) };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(mark).color(color));
            ui.label(RichText::new(format!("{:>7.1}s", n.t)).monospace().weak());
            ui.label(RichText::new(&n.text).color(color));
        });
    }
}

pub(super) fn show(p: &mut Panel, ui: &mut egui::Ui, status: Option<&Status>, hello: Option<&Hello>) {
    let Some(h) = hello else {
        ui.weak("waiting for the pet…");
        return;
    };
    let info = status.and_then(|s| s.test.as_ref());
    let mut out: Vec<Command> = Vec::new();
    ui.weak("Make him do one specific thing now. (This will later sit behind an \"achievements off for this session\" gate — the sandbox; for now achievements still count.)");
    ui.add_space(4.0);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        feedback(ui, &mut out, info);
    });
    ui.add_space(4.0);

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // --- Visitors -------------------------------------------------------------------
        section(ui, "Visitors", |ui| {
            if h.visitors.is_empty() {
                ui.weak("no visitors loaded (older pet, or data/npcs.json missing)");
                return;
            }
            p.visit_who = p.visit_who.min(h.visitors.len() - 1);
            let who = p.visit_who as u8;
            ui.horizontal_wrapped(|ui| {
                ui.label("Visitor");
                egui::ComboBox::from_id_salt("visit_who").selected_text(&h.visitors[p.visit_who].display).show_ui(ui, |ui| {
                    for (i, v) in h.visitors.iter().enumerate() {
                        ui.selectable_value(&mut p.visit_who, i, &v.display);
                    }
                });
                row(ui, &mut out, "Visit (talk)", "they drop by now (portal / walk-in); the knight comes over to talk; only the talk follows", Some(test(TestCommand::Visit { who, scene: Scene::TalkOnly })), Some(Placeable::Visitor { who, scene: Scene::TalkOnly }));
                row(ui, &mut out, "Visit (natural)", "they drop by now; whatever follows the talk is rolled as on a natural visit", Some(test(TestCommand::Visit { who, scene: Scene::Natural })), Some(Placeable::Visitor { who, scene: Scene::Natural }));
                for (scene, label) in &h.visitors[p.visit_who].scenes {
                    row(ui, &mut out, label, "they drop by now and this scene follows the talk (the random roll skipped; its hard conditions still apply, and the result line says why when one fails)", Some(test(TestCommand::Visit { who, scene: *scene })), Some(Placeable::Visitor { who, scene: *scene }));
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Every scene");
                for (i, v) in h.visitors.iter().enumerate() {
                    for (scene, label) in &v.scenes {
                        let tip = match (v.name.as_str(), scene) {
                            ("myla", _) => "Myla arrives beside a Geo deposit and sings while mining (needs a deposit on screen: Discoveries & Geo first)",
                            ("quirrel", _) => "Quirrel and the knight sit on a high window edge taking in the view (needs a settled window edge in the upper half of a monitor)",
                            ("cornifer", _) => "they sit (or stand) together with their maps open while he hums",
                            ("sly", _) => "Sly's nail lesson: demos, drills, praise, the bow",
                            ("zote", _) => "Zote challenges the knight to a spar",
                            (_, Scene::Race) => "a race to her needle across the windows (needs a settled finish he can reach on her monitor, else a duel)",
                            (_, Scene::Duel) => "a short duel she wins",
                            _ => "this scene after the talk",
                        };
                        row(ui, &mut out, label, tip, Some(test(TestCommand::Visit { who: i as u8, scene: *scene })), None);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                row(ui, &mut out, "Any visitor", "a random visitor, as the tray's \"Invite a visitor\"", Some(Command::InviteVisitor), None);
                row(ui, &mut out, "Dream nail", "he dream-nails the window he stands on (never while someone talks)", Some(Command::DreamNail), None);
            });
        });

        // --- Enemies --------------------------------------------------------------------
        section(ui, "Enemies", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, name) in h.enemies.iter().enumerate() {
                    row(ui, &mut out, name, "one of these wanders in now (only a fullscreen app, a visitor / the troupe, or a gauntlet holds it back)", Some(test(TestCommand::Enemy { kind: i as u8 })), Some(Placeable::Enemy(i as u8)));
                }
                row(ui, &mut out, "Any enemy", "a random one, as the tray's \"Send an enemy\"", Some(Command::SendEnemy), None);
                row(ui, &mut out, "Start a gauntlet", "three waves of enemies, then a big Geo reward; starts at the next good moment, ends early if you type or go fullscreen", Some(Command::StartGauntlet), None);
            });
        });

        // --- Grimm ----------------------------------------------------------------------
        section(ui, "Grimm", |ui| {
            ui.horizontal_wrapped(|ui| {
                row(ui, &mut out, "Summon the troupe master", "straight to the show, beside the knight, at the next good moment", Some(Command::SummonGrimm { lantern: false }), None);
                row(ui, &mut out, "Light a scarlet flame", "a dim flame on a settled ledge near him; YOU click it to summon Grimm right there (it waits ~30 min: build him an arena first)", Some(Command::SummonGrimm { lantern: true }), Some(Placeable::Flame));
                row(ui, &mut out, "Friendly visit", "he drops by with a flourish, bows, teleports around the knight and leaves (sometimes leaving the flame, with 8+ achievements)", Some(Command::GrimmVisit), None);
            });
        });

        // --- Pastimes & props -------------------------------------------------------------
        section(ui, "Pastimes & props", |ui| {
            ui.horizontal_wrapped(|ui| {
                row(ui, &mut out, "Training dummy", "a dummy on a ledge near him; he practises on it at once (combos, pogos, nail arts, fireballs)", Some(Command::Discover(DISCOVER_DUMMY)), Some(Placeable::Dummy));
                row(ui, &mut out, "Lumafly", "a lumafly drifts over his monitor's windows; he chases it", Some(Command::Discover(DISCOVER_LUMAFLY)), None);
                row(ui, &mut out, "The view", "he climbs to the end of a high, settled window edge and sits taking in the view (no waiting for restlessness)", Some(test(TestCommand::View)), None);
                row(ui, &mut out, "Stargazing", "he goes up to an open ledge and looks up a long while — at any hour from here (naturally only 21:00–05:00)", Some(test(TestCommand::Stargaze)), None);
            });
        });

        // --- Discoveries & Geo -------------------------------------------------------------
        section(ui, "Discoveries & Geo", |ui| {
            ui.horizontal_wrapped(|ui| {
                row(ui, &mut out, "Bench in his window", "reveal a found bench in the window he stands on (or the one under the cursor)", Some(Command::Discover(DISCOVER_BENCH)), None);
                if !h.bench_styles.is_empty() {
                    p.bench_style = p.bench_style.min(h.bench_styles.len() - 1);
                    egui::ComboBox::from_id_salt("bench_style").selected_text(&h.bench_styles[p.bench_style]).show_ui(ui, |ui| {
                        for (i, b) in h.bench_styles.iter().enumerate() {
                            ui.selectable_value(&mut p.bench_style, i, b);
                        }
                    });
                    row(ui, &mut out, "Bench", "a placed bench of this style where you click (rides its window)", None, Some(Placeable::Bench(p.bench_style as u8)));
                }
            });
            ui.horizontal_wrapped(|ui| {
                row(ui, &mut out, "Geo deposit", "a Geo deposit in the window he stands on; he mines it. Its rock's region follows the window's app (try different apps for Crossroads / City / Greenpath / Fungal art)", Some(Command::Discover(DISCOVER_DEPOSIT)), Some(Placeable::Deposit));
                row(ui, &mut out, "Shiny", "a glint near him; he picks it up (sometimes Geo pops out)", Some(Command::Discover(DISCOVER_SHINY)), Some(Placeable::Shiny));
                row(ui, &mut out, "Geo shower", "Geo tumbles out of a subtle portal over a settled ledge he can reach", Some(Command::Stakes(StakesCommand::SpillGeo)), Some(Placeable::GeoShower));
                row(ui, &mut out, "Spill Geo at the cursor", "a burst of 30 Geo out of the mouse cursor", Some(Command::Stakes(StakesCommand::DropGeo)), None);
                row(ui, &mut out, "Forget discoveries", "forget every find in windows, shinies, the dummy and lumaflies", Some(Command::ForgetDiscoveries), None);
            });
        });

        // --- The knight ---------------------------------------------------------------------
        section(ui, "Knight", |ui| {
            ui.horizontal_wrapped(|ui| {
                row(ui, &mut out, "Kill him", "he dies where he stands, leaves a Shade holding his Geo, and respawns", Some(Command::Stakes(StakesCommand::Die)), None);
                row(ui, &mut out, "Hunt the Shade now", "he goes after his Shade at once (a Dream Gate if no route reaches it)", Some(Command::Stakes(StakesCommand::HuntShade)), None);
                row(ui, &mut out, "Slug form", "Unn's slug crawl now (he has to be standing)", Some(test(TestCommand::Slug)), None);
                let emote = info.is_some_and(|i| i.emote_test);
                row(ui, &mut out, if emote { "Emote cycle: on" } else { "Emote cycle: off" }, "cycle every expression bubble above his head, 2.5 s each (as --emote-test)", Some(test(TestCommand::ToggleEmoteTest)), None);
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Spells");
                for (i, (_, name)) in SPELLS.iter().enumerate() {
                    row(ui, &mut out, name, "play this spell / nail art now (he has to be standing)", Some(Command::Stakes(StakesCommand::Cast(i as u8))), None);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Dream Gate");
                row(ui, &mut out, "Ready", "clear the Dream Gate's cooldown: the next prize no route reaches (a visitor, Geo, a deposit, a shiny) gets a gate at once", Some(test(TestCommand::DreamGateReady)), None);
                row(ui, &mut out, "Target", "put a shiny where you click and clear the cooldown: pick a spot he can't walk to (a window top with nothing to climb) and he gates to it", None, Some(Placeable::DreamGate));
            });
        });
    });
    for c in out {
        p.cmd(c);
    }
}
