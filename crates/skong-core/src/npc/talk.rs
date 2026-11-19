//! Conversation lines and their desktop-aware templates.
//!
//! Placeholders: `{title}` (a visible window's title, shortened), `{app}` (a friendly
//! name of a running app), `{daypart}` (morning / afternoon / evening / night). A line
//! whose placeholder cannot be filled makes its whole conversation unavailable.
use anyhow::{Result, bail};
use serde::Deserialize;

use crate::ai::rng::Rng;
use crate::desktop::DesktopSnapshot;

/// One line: plain text, or text plus the act the knight answers with.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Line {
    Plain(String),
    Full {
        say: String,
        /// Knight act names; `|` separates alternatives (first startable wins).
        #[serde(default)]
        react: Option<String>,
    },
}

impl Line {
    pub fn text(&self) -> &str {
        match self {
            Line::Plain(s) | Line::Full { say: s, .. } => s,
        }
    }

    pub fn react(&self) -> Option<&str> {
        match self {
            Line::Plain(_) => None,
            Line::Full { react, .. } => react.as_deref(),
        }
    }
}

const KEYS: [&str; 3] = ["title", "app", "daypart"];

/// Reject unknown or unbalanced placeholders at load time.
pub fn check_template(s: &str) -> Result<()> {
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        let Some(j) = rest[i..].find('}') else { bail!("unclosed '{{' in \"{s}\"") };
        let key = &rest[i + 1..i + j];
        if !KEYS.contains(&key) {
            bail!("unknown placeholder {{{key}}} in \"{s}\"");
        }
        rest = &rest[i + j + 1..];
    }
    Ok(())
}

/// What lines may talk about.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TalkContext {
    /// Visible window titles (shortened), most prominent first.
    pub titles: Vec<String>,
    /// Friendly app names.
    pub apps: Vec<String>,
    /// Local time (hours 0..24).
    pub hour: Option<f32>,
}

/// Processes that aren't "apps" to talk about.
const BORING: [&str; 8] = ["explorer", "skong", "skong-panel", "applicationframehost", "textinputhost", "shellexperiencehost", "searchhost", "startmenuexperiencehost"];

impl TalkContext {
    pub fn from_desktop(desk: &DesktopSnapshot, hour: Option<f32>) -> Self {
        let mut titles = Vec::new();
        let mut apps: Vec<String> = Vec::new();
        for w in &desk.windows {
            let proc = w.process.trim_end_matches(".exe").trim_end_matches(".EXE").to_lowercase();
            if BORING.contains(&proc.as_str()) {
                continue;
            }
            if let Some(t) = short_title(&w.title)
                && !titles.contains(&t)
            {
                titles.push(t);
            }
            let app = friendly_app(&w.process);
            if !app.is_empty() && !apps.contains(&app) {
                apps.push(app);
            }
        }
        Self { titles, apps, hour }
    }

    pub fn daypart(&self) -> Option<&'static str> {
        let h = self.hour?;
        Some(match h {
            h if (5.0..12.0).contains(&h) => "morning",
            h if (12.0..17.5).contains(&h) => "afternoon",
            h if (17.5..22.0).contains(&h) => "evening",
            _ => "night",
        })
    }

    /// Fill a template; `None` if a placeholder has no value. Picks among the first few
    /// titles / apps at random.
    pub fn fill(&self, template: &str, rng: &mut Rng) -> Option<String> {
        let mut out = String::with_capacity(template.len() + 16);
        let mut rest = template;
        while let Some(i) = rest.find('{') {
            out.push_str(&rest[..i]);
            let j = rest[i..].find('}')?;
            let key = &rest[i + 1..i + j];
            let pick = |v: &[String], rng: &mut Rng| -> Option<String> {
                let n = v.len().min(3);
                (n > 0).then(|| v[(rng.next_u32() as usize) % n].clone())
            };
            let val = match key {
                "title" => pick(&self.titles, rng)?,
                "app" => pick(&self.apps, rng)?,
                "daypart" => self.daypart()?.to_string(),
                _ => return None,
            };
            out.push_str(&val);
            rest = &rest[i + j + 1..];
        }
        out.push_str(rest);
        Some(out)
    }

    /// A random conversation whose every line can be filled, as (text, react) pairs.
    pub fn pick(&self, convos: &[Vec<Line>], rng: &mut Rng) -> Option<Vec<(String, Option<String>)>> {
        let start = rng.next_u32() as usize;
        (0..convos.len()).find_map(|k| {
            let c = &convos[(start + k) % convos.len()];
            c.iter().map(|l| Some((self.fill(l.text(), rng)?, l.react().map(str::to_string)))).collect()
        })
    }
}

/// A window title worth saying out loud: trimmed of the "- App" suffix, at most ~28 chars.
pub fn short_title(title: &str) -> Option<String> {
    let t = title.trim();
    // "Document - App Name" -> "Document" (the part users recognise).
    let main = t.split(" - ").next().unwrap_or(t).split(" — ").next().unwrap_or(t).trim();
    if main.chars().count() < 3 {
        return None;
    }
    const MAX: usize = 28;
    if main.chars().count() <= MAX {
        return Some(main.to_string());
    }
    let cut: String = main.chars().take(MAX - 1).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(a, _)| a).trim_end();
    Some(format!("{cut}…"))
}

/// "msedge.exe" → "Edge", "Code.exe" → "Code", "WINWORD.EXE" → "Word".
pub fn friendly_app(process: &str) -> String {
    let p = process.trim_end_matches(".exe").trim_end_matches(".EXE");
    let known = [
        ("msedge", "Edge"),
        ("chrome", "Chrome"),
        ("firefox", "Firefox"),
        ("winword", "Word"),
        ("excel", "Excel"),
        ("powerpnt", "PowerPoint"),
        ("code", "Code"),
        ("windowsterminal", "Terminal"),
        ("devenv", "Visual Studio"),
        ("spotify", "Spotify"),
        ("discord", "Discord"),
        ("slack", "Slack"),
        ("notepad", "Notepad"),
        ("explorer", "Explorer"),
        ("steam", "Steam"),
        ("obsidian", "Obsidian"),
        ("teams", "Teams"),
        ("ms-teams", "Teams"),
    ];
    let lower = p.to_lowercase();
    if let Some((_, n)) = known.iter().find(|(k, _)| *k == lower) {
        return n.to_string();
    }
    let mut c = p.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> TalkContext {
        TalkContext { titles: vec!["Budget".into()], apps: vec!["Excel".into()], hour: Some(20.0) }
    }

    #[test]
    fn fills_placeholders() {
        let mut rng = Rng::new(1);
        assert_eq!(ctx().fill("A fine {daypart} for {app}: '{title}'!", &mut rng).as_deref(), Some("A fine evening for Excel: 'Budget'!"));
        let empty = TalkContext::default();
        assert_eq!(empty.fill("no {title} here", &mut rng), None);
        assert_eq!(empty.fill("plain", &mut rng).as_deref(), Some("plain"));
    }

    #[test]
    fn picks_a_fillable_conversation() {
        let convos = vec![vec![Line::Plain("About {title}".into())], vec![Line::Full { say: "Hello".into(), react: Some("look_up".into()) }]];
        let mut rng = Rng::new(3);
        for _ in 0..10 {
            let c = TalkContext::default().pick(&convos, &mut rng).expect("second is always fillable");
            assert_eq!(c, vec![("Hello".to_string(), Some("look_up".to_string()))]);
        }
        assert!(TalkContext::default().pick(&convos[..1], &mut rng).is_none());
    }

    #[test]
    fn checks_templates() {
        assert!(check_template("{title} and {daypart}").is_ok());
        assert!(check_template("{name}").is_err());
        assert!(check_template("oops {title").is_err());
    }

    #[test]
    fn shortens_titles_and_names_apps() {
        assert_eq!(short_title("notes.txt - Notepad").as_deref(), Some("notes.txt"));
        assert_eq!(short_title("  ").as_deref(), None);
        let long = short_title("A really quite remarkably long document title indeed").expect("some");
        assert!(long.ends_with('…') && long.chars().count() <= 28, "{long}");
        assert_eq!(friendly_app("msedge.exe"), "Edge");
        assert_eq!(friendly_app("blender.exe"), "Blender");
    }

    #[test]
    fn dayparts() {
        let at = |h: f32| TalkContext { hour: Some(h), ..Default::default() }.daypart();
        assert_eq!((at(8.0), at(13.0), at(19.0), at(2.0)), (Some("morning"), Some("afternoon"), Some("evening"), Some("night")));
    }
}
