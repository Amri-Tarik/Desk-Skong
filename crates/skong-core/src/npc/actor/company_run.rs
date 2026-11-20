//! The visitor's side of a calm scene (see `crate::npc::company`): the invitation, sitting
//! together (their scene clip and a quiet line now and then, counted only while the knight
//! is beside them), the parting line and a last clip, then lingering and leaving as usual.
use super::{Npc, NpcEnv, NpcEvent, NpcSet, State, once_secs};
use crate::ai::{CompanyCue, CompanyStage};

/// The knight may wander off this long before the scene is given up (s).
const AWAY_SECS: f32 = 10.0;

/// A scene to start once the conversation is over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Plan {
    face_x: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Invite,
    Together,
    Parting,
    /// The scene's closing clip (Quirrel turning back from the view).
    End,
}

/// A scene in progress.
#[derive(Clone, Debug)]
pub(super) struct Company {
    stage: Stage,
    t: f32,
    /// Length of the current line / clip.
    len: f32,
    /// Seconds together still to go (only counted while the knight is beside them).
    left: f32,
    line_in: f32,
    away: f32,
    /// What they turn to while together (Myla's deposit); `None`: the knight.
    face_x: Option<f32>,
}

impl Company {
    pub(super) fn cue(&self, set: &NpcSet, def: usize) -> Option<CompanyCue> {
        let d = set.defs[def].company.as_ref()?;
        let stage = match self.stage {
            Stage::Invite => CompanyStage::Invite,
            Stage::Together => CompanyStage::Together,
            Stage::Parting | Stage::End => CompanyStage::Parting,
        };
        Some(CompanyCue { act: d.knight.clone(), stage })
    }
}

impl Npc {
    /// Invite the knight to stay a while once the conversation (not yet started) is over
    /// (needs a `company` def). `face_x`: what they turn to meanwhile (Myla's deposit).
    pub fn plan_company(&mut self, set: &NpcSet, face_x: Option<f32>) {
        self.company = set.defs[self.def].company.is_some().then_some(Plan { face_x });
    }

    /// A calm scene is planned or on.
    pub fn company_planned(&self) -> bool {
        self.company.is_some() || matches!(self.state, State::Company(_))
    }

    pub(super) fn start_company(&mut self, set: &NpcSet, events: &mut Vec<NpcEvent>) -> State {
        let plan = self.company.take();
        let (Some(plan), Some(def)) = (plan, set.defs[self.def].company.as_ref()) else {
            return State::Lingering { left: self.rng.range(3.0, 6.0) };
        };
        let text = def.invite[self.rng.next_u32() as usize % def.invite.len()].clone();
        let len = crate::bubble::read_secs(&text);
        let (talk, _) = self.talk_clips(set);
        self.anim.play(set.lib(self.def), talk, true);
        events.push(NpcEvent::Say { text, secs: len });
        let left = self.rng.range(def.secs[0], def.secs[1]);
        let line_in = self.rng.range(def.every[0], def.every[1]) * 0.5;
        State::Company(Box::new(Company { stage: Stage::Invite, t: 0.0, len, left, line_in, away: 0.0, face_x: plan.face_x }))
    }

    /// Cut the scene short (the knight was called away): a parting word, then leave as usual.
    pub fn end_company(&mut self, set: &NpcSet, events: &mut Vec<NpcEvent>) {
        if matches!(self.state, State::Company(_)) {
            self.state = self.abandon_company(set, events);
        }
    }

    fn abandon_company(&mut self, set: &NpcSet, events: &mut Vec<NpcEvent>) -> State {
        events.push(NpcEvent::LineDone);
        events.push(NpcEvent::CompanyOver { finished: false });
        self.anim.play(set.lib(self.def), set.clips[self.def].idle, false);
        let mut left = self.rng.range(1.5, 3.0);
        if let Some(def) = set.defs[self.def].company.as_ref() {
            let text = def.abandon[self.rng.next_u32() as usize % def.abandon.len()].clone();
            let secs = crate::bubble::read_secs(&text);
            left += secs;
            events.push(NpcEvent::Say { text, secs });
        }
        State::Lingering { left }
    }

    pub(super) fn company_tick(&mut self, mut c: Box<Company>, dt: f32, set: &NpcSet, env: &NpcEnv, events: &mut Vec<NpcEvent>) -> State {
        let (Some(def), Some(clips)) = (set.defs[self.def].company.as_ref(), set.clips[self.def].company) else {
            return State::Lingering { left: 2.0 };
        };
        let lib = set.lib(self.def);
        c.t += dt;
        let reach = set.defs[self.def].stand_off * env.ppu + 1.2 * env.ppu;
        let near = (env.knight.x - self.pos.x).abs() <= reach && (env.knight.y - self.pos.y).abs() < 1.5 * env.ppu;
        c.away = if near { 0.0 } else { c.away + dt };
        if matches!(c.stage, Stage::Invite | Stage::Together) && c.away > AWAY_SECS {
            return self.abandon_company(set, events);
        }
        let face_knight = |me: &mut Self| {
            if (env.knight.x - me.pos.x).abs() > 2.0 {
                me.looking_right = env.knight.x > me.pos.x;
            }
        };
        match c.stage {
            Stage::Invite => {
                face_knight(self);
                if c.t >= c.len {
                    events.push(NpcEvent::LineDone);
                    self.anim.play(lib, clips.clip, true);
                    c.stage = Stage::Together;
                    c.t = 0.0;
                }
            }
            Stage::Together => {
                match c.face_x {
                    Some(x) => self.looking_right = x > self.pos.x,
                    None if near => face_knight(self),
                    None => {}
                }
                if near {
                    c.left -= dt;
                    c.line_in -= dt;
                }
                if c.left <= 0.0 {
                    let text = def.parting[self.rng.next_u32() as usize % def.parting.len()].clone();
                    c.len = crate::bubble::read_secs(&text);
                    let (talk, _) = self.talk_clips(set);
                    face_knight(self);
                    self.anim.play(lib, talk, true);
                    events.push(NpcEvent::Say { text, secs: c.len });
                    c.stage = Stage::Parting;
                    c.t = 0.0;
                } else if c.line_in <= 0.0 && !def.lines.is_empty() {
                    let text = def.lines[self.rng.next_u32() as usize % def.lines.len()].clone();
                    let secs = crate::bubble::read_secs(&text).min(4.0);
                    events.push(NpcEvent::Say { text, secs });
                    c.line_in = self.rng.range(def.every[0], def.every[1]) + secs;
                }
            }
            Stage::Parting => {
                if c.t >= c.len {
                    events.push(NpcEvent::LineDone);
                    match clips.end {
                        Some(end) => {
                            self.anim.play(lib, end, true);
                            c.len = once_secs(lib, end).max(0.05);
                            c.stage = Stage::End;
                            c.t = 0.0;
                        }
                        None => return self.company_done(set, events),
                    }
                }
            }
            Stage::End => {
                if c.t >= c.len {
                    return self.company_done(set, events);
                }
            }
        }
        State::Company(c)
    }

    fn company_done(&mut self, set: &NpcSet, events: &mut Vec<NpcEvent>) -> State {
        events.push(NpcEvent::CompanyOver { finished: true });
        self.anim.play(set.lib(self.def), set.clips[self.def].idle, false);
        State::Lingering { left: self.rng.range(3.0, 5.0) }
    }
}
