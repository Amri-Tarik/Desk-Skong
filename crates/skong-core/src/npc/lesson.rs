//! Lessons: a visitor who teaches (Great Nailsage Sly) follows the conversation with a
//! short training session. Each drill is announced (and maybe demonstrated with the
//! visitor's own clips), the knight practises it (the brain's `Train` behaviour reports
//! back), and the visitor answers with praise or a correction. A closing line ends it.
//!
//! Definitions come from `npcs.json` (`lesson`, `demos`); [`Lesson::plan`] turns one into
//! a concrete session with every line filled for the current desktop.
use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::ai::rng::Rng;

use super::talk::{Line, TalkContext, check_template};

/// One step of a demonstration: a clip of the visitor's library, played once (or for
/// `secs` when it loops).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum DemoStep {
    Clip(String),
    Timed { clip: String, secs: f32 },
}

impl DemoStep {
    pub fn clip(&self) -> &str {
        match self {
            DemoStep::Clip(c) | DemoStep::Timed { clip: c, .. } => c,
        }
    }

    pub fn secs(&self) -> Option<f32> {
        match self {
            DemoStep::Clip(_) => None,
            DemoStep::Timed { secs, .. } => Some(*secs),
        }
    }
}

/// A drill the teacher may ask for.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrillDef {
    /// What the knight practises: `slash`, `up_slash`, `down_slash`, `dash`, `jump`, or the
    /// name of a knight act (`great_slash`, `cyclone_slash`, `dash_slash`...).
    pub drill: String,
    /// Instruction lines (one is picked).
    pub say: Vec<String>,
    /// Demonstration (a key of the visitor's `demos`) shown after the instruction.
    #[serde(default)]
    pub demo: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LessonDef {
    /// The first drill is always taught; the rest are a random selection in listed order.
    pub drills: Vec<DrillDef>,
    /// How many drills per session (random in range).
    #[serde(default = "count")]
    pub count: [usize; 2],
    /// Demonstration played once as the lesson starts (drawing the nail...).
    #[serde(default)]
    pub open_demo: Option<String>,
    /// Clip held while teaching and watching (default: idle).
    #[serde(default)]
    pub stance: Option<String>,
    /// Seconds the teacher waits for the knight to finish a drill.
    #[serde(default = "practice_secs")]
    pub practice_secs: f32,
    /// Feedback for a drill done well (sometimes a correction anyway: teachers are strict).
    pub praise: Vec<Line>,
    /// Feedback for a sloppy or unfinished drill.
    pub critique: Vec<Line>,
    /// The last word before leaving (the knight's reaction may be a bow).
    pub closing: Vec<Line>,
    /// Said when the lesson is cut short (the user needs the screen, the knight wandered off).
    #[serde(default)]
    pub abandon: Vec<String>,
}

fn count() -> [usize; 2] {
    [3, 5]
}
fn practice_secs() -> f32 {
    14.0
}

impl LessonDef {
    pub fn validate(&self, demos: &dyn Fn(&str) -> bool) -> Result<()> {
        if self.drills.is_empty() || self.praise.is_empty() || self.critique.is_empty() || self.closing.is_empty() {
            bail!("lesson needs drills, praise, critique and closing lines");
        }
        if self.count[0] == 0 || self.count[1] < self.count[0] {
            bail!("lesson: bad count range");
        }
        if self.practice_secs.is_nan() || self.practice_secs <= 0.0 {
            bail!("lesson: practice_secs must be positive");
        }
        for d in &self.drills {
            if d.say.is_empty() {
                bail!("lesson drill '{}': no instruction line", d.drill);
            }
            for s in &d.say {
                check_template(s).with_context(|| format!("lesson drill '{}'", d.drill))?;
            }
        }
        for name in self.drills.iter().filter_map(|d| d.demo.as_deref()).chain(self.open_demo.as_deref()) {
            if !demos(name) {
                bail!("lesson: unknown demo '{name}'");
            }
        }
        let lines = self.praise.iter().chain(&self.critique).chain(&self.closing).map(Line::text);
        for s in lines.chain(self.abandon.iter().map(String::as_str)) {
            check_template(s).context("lesson line")?;
        }
        Ok(())
    }
}

/// A drill of a planned session.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedDrill {
    pub drill: String,
    pub say: String,
    pub demo: Option<String>,
}

/// A concrete session: drills in order and filled feedback pools.
#[derive(Clone, Debug, PartialEq)]
pub struct Lesson {
    pub drills: Vec<PlannedDrill>,
    pub praise: Vec<(String, Option<String>)>,
    pub critique: Vec<(String, Option<String>)>,
    pub closing: Vec<(String, Option<String>)>,
    pub abandon: Vec<String>,
    pub open_demo: Option<String>,
    pub practice_secs: f32,
}

impl Lesson {
    /// Pick the drills and fill every line. `None` if nothing could be filled (e.g. every
    /// line names a window and there is none).
    pub fn plan(def: &LessonDef, ctx: &TalkContext, rng: &mut Rng) -> Option<Lesson> {
        let fill_one = |pool: &[String], rng: &mut Rng| -> Option<String> {
            let start = rng.next_u32() as usize;
            (0..pool.len()).find_map(|k| ctx.fill(&pool[(start + k) % pool.len()], rng))
        };
        let lines = |pool: &[Line], rng: &mut Rng| -> Vec<(String, Option<String>)> {
            pool.iter().filter_map(|l| Some((ctx.fill(l.text(), rng)?, l.react().map(str::to_string)))).collect()
        };
        let want = (def.count[0] + (rng.next_u32() as usize) % (def.count[1] - def.count[0] + 1)).min(def.drills.len());
        // The first drill always, then a random subset of the rest, kept in listed order.
        let mut rest: Vec<usize> = (1..def.drills.len()).collect();
        for i in (1..rest.len()).rev() {
            rest.swap(i, rng.next_u32() as usize % (i + 1));
        }
        let mut picked: Vec<usize> = std::iter::once(0).chain(rest.into_iter().take(want.saturating_sub(1))).collect();
        picked.sort_unstable();
        let drills: Vec<PlannedDrill> = picked
            .into_iter()
            .filter_map(|i| {
                let d = &def.drills[i];
                Some(PlannedDrill { drill: d.drill.clone(), say: fill_one(&d.say, rng)?, demo: d.demo.clone() })
            })
            .collect();
        let lesson = Lesson {
            drills,
            praise: lines(&def.praise, rng),
            critique: lines(&def.critique, rng),
            closing: lines(&def.closing, rng),
            abandon: def.abandon.iter().filter_map(|s| ctx.fill(s, rng)).collect(),
            open_demo: def.open_demo.clone(),
            practice_secs: def.practice_secs,
        };
        let ok = !lesson.drills.is_empty() && !lesson.praise.is_empty() && !lesson.critique.is_empty() && !lesson.closing.is_empty();
        ok.then_some(lesson)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> LessonDef {
        let d = |n: &str| DrillDef { drill: n.into(), say: vec![format!("Now {n}.")], demo: None };
        LessonDef {
            drills: vec![d("slash"), d("up_slash"), d("dash"), d("jump"), d("great_slash")],
            count: [3, 4],
            open_demo: None,
            stance: None,
            practice_secs: 10.0,
            praise: vec![Line::Plain("Good.".into())],
            critique: vec![Line::Plain("Again, on {title}.".into()), Line::Plain("Sloppy.".into())],
            closing: vec![Line::Full { say: "Enough.".into(), react: Some("bow_down".into()) }],
            abandon: vec![],
        }
    }

    #[test]
    fn plans_the_first_drill_then_a_subset_in_order() {
        let order = ["slash", "up_slash", "dash", "jump", "great_slash"];
        for seed in 0..30 {
            let l = Lesson::plan(&def(), &TalkContext::default(), &mut Rng::new(seed)).expect("plan");
            assert!((3..=4).contains(&l.drills.len()), "{}", l.drills.len());
            assert_eq!(l.drills[0].drill, "slash");
            let idx: Vec<usize> = l.drills.iter().map(|d| order.iter().position(|o| *o == d.drill).unwrap()).collect();
            assert!(idx.windows(2).all(|w| w[0] < w[1]), "listed order, no repeats: {idx:?}");
            // Unfillable lines are dropped from the pools.
            assert_eq!(l.critique, vec![("Sloppy.".to_string(), None)]);
            assert_eq!(l.closing[0].1.as_deref(), Some("bow_down"));
        }
    }

    #[test]
    fn validates() {
        let ok = |_: &str| true;
        assert!(def().validate(&ok).is_ok());
        let mut bad = def();
        bad.drills[1].demo = Some("nope".into());
        assert!(bad.validate(&|n: &str| n == "slash").is_err());
        let mut bad = def();
        bad.closing.clear();
        assert!(bad.validate(&ok).is_err());
    }
}
