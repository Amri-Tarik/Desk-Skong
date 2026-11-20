//! First-run art setup (`skong-panel --setup`): finds the player's Hollow Knight install,
//! reads Desk-Skong's sprites from it once (`skong-extract`) and saves them into the user's
//! data folder. The pet starts this window and waits for it when the art is missing; the
//! tray and the panel open it again for "Re-extract game art".
//!
//! Exit code 0: the art is in place. Anything else: the player closed the window first.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, RichText};

pub struct Options {
    /// Desk-Skong's metadata (`data/hk`).
    pub data: PathBuf,
    /// The Vanilla skin folder to write.
    pub out: PathBuf,
    /// The install remembered from last time, tried first.
    pub game: Option<PathBuf>,
    /// Re-extracting while the pet runs (stay open when done, instead of closing).
    pub again: bool,
}

pub const NEEDS: &str = "Desk-Skong needs your own copy of Hollow Knight.";
pub const EXPLAIN: &str = "It reads the game's sprites from your installation, once, and keeps them on this PC only.";
pub const LEGAL: &str = "Requires a legitimately owned copy of Hollow Knight. Nothing from the game is distributed with Desk-Skong. \
    Desk-Skong is a fan project, not affiliated with or endorsed by Team Cherry; Hollow Knight and its art belong to Team Cherry.";

#[derive(Default)]
struct Work {
    fraction: f32,
    label: String,
    result: Option<Result<String, String>>,
}

enum Stage {
    /// Nothing found yet (or the pick wasn't the game): ask for the exe.
    Ask(Option<String>),
    Working { game: PathBuf, work: Arc<Mutex<Work>> },
    Done(String),
    Failed { game: PathBuf, error: String },
}

struct Setup {
    opt: Options,
    stage: Stage,
    ok: Arc<AtomicBool>,
}

pub fn run(opt: Options) -> i32 {
    let ok = Arc::new(AtomicBool::new(false));
    let ok2 = ok.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Desk-Skong: set up the art").with_inner_size([560.0, 340.0]).with_resizable(false),
        ..Default::default()
    };
    let r = eframe::run_native(
        "Desk-Skong setup",
        options,
        Box::new(move |cc| {
            let found = opt.game.as_deref().and_then(skong_extract::install::data_dir).or_else(skong_extract::install::find);
            let mut s = Setup { opt, stage: Stage::Ask(None), ok: ok2 };
            if let Some(game) = found {
                s.start(game, &cc.egui_ctx);
            }
            Ok(Box::new(s))
        }),
    );
    if let Err(e) = r {
        eprintln!("setup window: {e}");
    }
    if ok.load(Ordering::SeqCst) { 0 } else { 1 }
}

impl Setup {
    fn start(&mut self, game: PathBuf, ctx: &egui::Context) {
        let work = Arc::new(Mutex::new(Work { label: "Looking through the game files…".into(), ..Default::default() }));
        let (w, ctx, data, out, g) = (work.clone(), ctx.clone(), self.opt.data.clone(), self.opt.out.clone(), game.clone());
        std::thread::spawn(move || {
            let progress = |p: skong_extract::Progress| {
                if let Ok(mut w) = w.lock() {
                    w.fraction = p.fraction();
                    w.label = match p {
                        skong_extract::Progress::Searching { file, files } => format!("Looking through the game files… {file}/{files}"),
                        skong_extract::Progress::Writing { done, total } => format!("Saving images… {done}/{total}"),
                    };
                }
                ctx.request_repaint();
            };
            // A panic in the reader must end up as a message, not a vanished window.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| extract(&data, &g, &out, &progress)))
                .unwrap_or_else(|_| Err("the extractor crashed (please report this with your game version)".into()));
            if let Ok(mut w) = w.lock() {
                w.result = Some(result);
            }
            ctx.request_repaint();
        });
        self.stage = Stage::Working { game, work };
    }

    fn locate(&mut self, ctx: &egui::Context) {
        let Some(pick) = pick_exe() else { return };
        match skong_extract::install::data_dir(&pick) {
            Some(game) => self.start(game, ctx),
            None => self.stage = Stage::Ask(Some(format!("That doesn't look like Hollow Knight: {}. Pick hollow_knight.exe in the game's folder.", pick.display()))),
        }
    }
}

fn extract(data: &Path, game: &Path, out: &Path, progress: &(dyn Fn(skong_extract::Progress) + Sync)) -> Result<String, String> {
    let plan = skong_extract::plan(data, &data.join("../props.json")).map_err(|e| format!("Desk-Skong's own data is missing or broken ({e:#})"))?;
    let r = skong_extract::extract(game, &plan, out, progress).map_err(|e| format!("{e:#}"))?;
    let mut msg = format!("Done: {} images in {:.1} s.", r.written.len(), r.seconds);
    if !r.missing.is_empty() {
        msg.push_str(&format!(" Not in this version of the game (drawn as stand-ins): {}.", r.missing.join(", ")));
    }
    Ok(msg)
}

impl eframe::App for Setup {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Finished work moves the stage on.
        if let Stage::Working { game, work } = &self.stage {
            let result = work.lock().ok().and_then(|mut w| w.result.take());
            match result {
                Some(Ok(msg)) => {
                    self.ok.store(true, Ordering::SeqCst);
                    if !self.opt.again {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    self.stage = Stage::Done(msg);
                }
                Some(Err(error)) => self.stage = Stage::Failed { game: game.clone(), error },
                None => {}
            }
        }

        let mut action: Option<Action> = None;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new(NEEDS).heading());
            ui.label(EXPLAIN);
            ui.add_space(4.0);
            ui.label(RichText::new(LEGAL).small().weak());
            ui.separator();
            match &self.stage {
                Stage::Ask(problem) => {
                    match problem {
                        Some(p) => ui.label(RichText::new(p).color(Color32::from_rgb(230, 160, 60))),
                        None => ui.label("Hollow Knight wasn't found in your Steam libraries or the usual GOG, Epic and Xbox folders."),
                    };
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Locate hollow_knight.exe…").clicked() {
                            action = Some(Action::Locate);
                        }
                        if ui.button("Quit").clicked() {
                            action = Some(Action::Quit);
                        }
                    });
                }
                Stage::Working { game, work } => {
                    let (fraction, label) = work.lock().map(|w| (w.fraction, w.label.clone())).unwrap_or_default();
                    ui.label(format!("Found Hollow Knight: {}", game.parent().unwrap_or(game).display()));
                    ui.add(egui::ProgressBar::new(fraction).show_percentage());
                    ui.label(RichText::new(label).weak());
                }
                Stage::Done(msg) => {
                    ui.label(RichText::new(msg).color(Color32::from_rgb(90, 200, 110)));
                    ui.label(RichText::new(format!("Saved in {}", self.opt.out.display())).small().weak());
                    if self.opt.again {
                        ui.label("The new art is used from the next time it's loaded; restart Desk-Skong to see all of it.");
                    }
                    if ui.button("Close").clicked() {
                        action = Some(Action::Quit);
                    }
                }
                Stage::Failed { error, .. } => {
                    ui.label(RichText::new("Couldn't read the art from the game.").color(Color32::from_rgb(230, 110, 90)).strong());
                    ui.label(RichText::new(error).small());
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Try again").clicked() {
                            action = Some(Action::Retry);
                        }
                        if ui.button("Locate hollow_knight.exe…").clicked() {
                            action = Some(Action::Locate);
                        }
                        if ui.button("Quit").clicked() {
                            action = Some(Action::Quit);
                        }
                    });
                }
            }
        });
        match action {
            Some(Action::Locate) => self.locate(&ctx),
            Some(Action::Retry) => {
                if let Stage::Failed { game, .. } = &self.stage {
                    let game = game.clone();
                    self.start(game, &ctx);
                }
            }
            Some(Action::Quit) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            None => {}
        }
    }
}

enum Action {
    Locate,
    Retry,
    Quit,
}

/// The native "open file" dialog, filtered to `hollow_knight.exe`.
#[cfg(windows)]
fn pick_exe() -> Option<PathBuf> {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW};
    use windows::core::{PCWSTR, PWSTR};
    let filter: Vec<u16> = "Hollow Knight (hollow_knight.exe)\0hollow_knight.exe\0Programs (*.exe)\0*.exe\0\0".encode_utf16().collect();
    let title: Vec<u16> = "Locate hollow_knight.exe\0".encode_utf16().collect();
    let mut file = vec![0u16; 1024];
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file.as_mut_ptr()),
        nMaxFile: file.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    // SAFETY: every pointer in `ofn` points into buffers that outlive the call.
    if !unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() {
        return None;
    }
    let n = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    Some(PathBuf::from(String::from_utf16_lossy(&file[..n])))
}

#[cfg(not(windows))]
fn pick_exe() -> Option<PathBuf> {
    None
}
