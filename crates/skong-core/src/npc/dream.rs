//! Dream Nail thoughts: when the knight dream-nails a window, it "thinks" a short line
//! picked from rules in `data/npcs.json` (`dream`), matched on the window's process name
//! or title, with the usual `{title}` / `{app}` / `{daypart}` placeholders bound to *that*
//! window.
use serde::Deserialize;

use crate::ai::rng::Rng;
use crate::desktop::WindowInfo;

use super::talk::{TalkContext, friendly_app, short_title};

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DreamRule {
    /// Lowercase substrings of the process name or title; empty = matches anything.
    #[serde(default, rename = "match")]
    pub matches: Vec<String>,
    pub thoughts: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(transparent)]
pub struct DreamRules(pub Vec<DreamRule>);

impl DreamRules {
    /// A thought for `w`: from the first matching specific rule (sometimes from the
    /// generic pool instead, for variety). `None` only when there are no rules at all.
    pub fn thought(&self, w: &WindowInfo, hour: Option<f32>, rng: &mut Rng) -> Option<String> {
        let proc = w.process.to_lowercase();
        let title = w.title.to_lowercase();
        let hit = |r: &&DreamRule| r.matches.iter().any(|m| proc.contains(m.as_str()) || title.contains(m.as_str()));
        let specific = self.0.iter().find(hit);
        let generic: Vec<&DreamRule> = self.0.iter().filter(|r| r.matches.is_empty()).collect();
        let ctx = TalkContext { titles: short_title(&w.title).into_iter().collect(), apps: vec![friendly_app(&w.process)], hour };
        let pool: Vec<&String> = match specific {
            Some(r) if generic.is_empty() || rng.chance(0.75) => r.thoughts.iter().collect(),
            _ => generic.iter().flat_map(|r| r.thoughts.iter()).collect(),
        };
        // Try a few: a template may need a title the window doesn't have.
        let start = rng.next_u32() as usize;
        (0..pool.len()).find_map(|k| ctx.fill(pool[(start + k) % pool.len()], rng))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(process: &str, title: &str) -> WindowInfo {
        WindowInfo {
            id: 1,
            rect: crate::geom::RectI::new(0, 0, 100, 100),
            title: title.into(),
            class: String::new(),
            process: process.into(),
            pid: 1,
            maximized: false,
            fullscreen: false,
            foreground: false,
            topmost: false,
        }
    }

    #[test]
    fn matches_process_then_falls_back() {
        let rules = DreamRules(vec![
            DreamRule { matches: vec!["code".into()], thoughts: vec!["semicolons in {app}...".into()] },
            DreamRule { matches: vec![], thoughts: vec!["'{title}'... how odd.".into()] },
        ]);
        let mut rng = Rng::new(5);
        let mut saw_specific = false;
        for _ in 0..20 {
            let t = rules.thought(&win("Code.exe", "main.rs - proj"), None, &mut rng).expect("thought");
            saw_specific |= t == "semicolons in Code...";
            assert!(t == "semicolons in Code..." || t == "'main.rs'... how odd.", "{t}");
        }
        assert!(saw_specific);
        // No title: the generic template can't fill, nothing else matches.
        assert_eq!(rules.thought(&win("other.exe", ""), None, &mut rng), None);
    }
}
