//! Scenes the test panel can ask a visitor for after the talk (the random rolls skipped,
//! the hard conditions kept), and why one can't run, in words for the panel.
use super::NpcDef;
use super::company::CompanyNeeds;

/// What follows the talk, forced from the test panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForcedScene {
    /// Only the talk: no lesson, spar, calm scene or challenge.
    TalkOnly,
    /// Sly's lesson.
    Lesson,
    /// Zote's spar.
    Spar,
    /// The calm scene (Cornifer's maps, Quirrel's view, Myla's song).
    Company,
    /// Hornet's race / duel.
    Race,
    Duel,
}

impl ForcedScene {
    pub fn label(self) -> &'static str {
        match self {
            ForcedScene::TalkOnly => "a talk",
            ForcedScene::Lesson => "a lesson",
            ForcedScene::Spar => "a spar",
            ForcedScene::Company => "a calm scene",
            ForcedScene::Race => "a race",
            ForcedScene::Duel => "a duel",
        }
    }
}

/// Does this visitor have `scene` at all?
pub fn supports(def: &NpcDef, scene: ForcedScene) -> Result<(), String> {
    let has = match scene {
        ForcedScene::TalkOnly => true,
        ForcedScene::Lesson => def.lesson.is_some(),
        ForcedScene::Spar => def.spar.is_some(),
        ForcedScene::Company => def.company.is_some(),
        ForcedScene::Race | ForcedScene::Duel => def.challenge.is_some(),
    };
    if has { Ok(()) } else { Err(format!("{} has no {} scene", def.display, scene.label().trim_start_matches("a "))) }
}

/// What a calm scene of `needs` is called in "can't …" lines.
pub fn company_verb(needs: CompanyNeeds) -> &'static str {
    match needs {
        CompanyNeeds::Any => "stay a while",
        CompanyNeeds::Seat => "sit down with him",
        CompanyNeeds::View => "take in the view",
        CompanyNeeds::Deposit => "sing by a Geo deposit",
    }
}

/// The window edge beside a visitor, as a calm scene with a seat sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatCheck {
    /// No window edge under both of them (the floor, or the knight off its end).
    None,
    /// There is one, but it moved / appeared in the last few seconds.
    Unsettled,
    /// Settled, but low on its monitor (no view).
    Low,
    /// Settled and high.
    High,
}

/// The world, as a forced calm scene's hard conditions see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompanyWorld {
    pub fullscreen: bool,
    pub typing: bool,
    /// The deposit the visitor arrived beside is still on screen.
    pub deposit: bool,
    pub seat: SeatCheck,
}

/// Why a calm scene of `needs` can't run now (`None`: it can), e.g. "Myla can't sing by a
/// Geo deposit: no Geo deposit beside her on screen".
pub fn company_blocker(who: &str, needs: CompanyNeeds, w: CompanyWorld) -> Option<String> {
    let why = if w.fullscreen {
        "a fullscreen app is up"
    } else if w.typing {
        "you're typing"
    } else {
        match (needs, w.seat) {
            (CompanyNeeds::Any, _) => return None,
            (CompanyNeeds::Deposit, _) if w.deposit => return None,
            (CompanyNeeds::Deposit, _) => "no Geo deposit beside them on screen (reveal one first: Discoveries & Geo)",
            (CompanyNeeds::Seat | CompanyNeeds::View, SeatCheck::None) => "no window edge under both of them to sit on",
            (CompanyNeeds::Seat | CompanyNeeds::View, SeatCheck::Unsettled) => "that window edge moved in the last 15 s (it has to settle)",
            (CompanyNeeds::Seat, SeatCheck::Low | SeatCheck::High) | (CompanyNeeds::View, SeatCheck::High) => return None,
            (CompanyNeeds::View, SeatCheck::Low) => "that window edge is in the lower half of its monitor (no view)",
        }
    };
    Some(format!("{who} can't {}: {why}", company_verb(needs)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: CompanyWorld = CompanyWorld { fullscreen: false, typing: false, deposit: false, seat: SeatCheck::None };

    #[test]
    fn calm_scenes_keep_their_hard_conditions() {
        assert_eq!(company_blocker("Cornifer", CompanyNeeds::Any, OK), None);
        let myla = company_blocker("Myla", CompanyNeeds::Deposit, OK).unwrap();
        assert!(myla.starts_with("Myla can't sing by a Geo deposit: no Geo deposit"), "{myla}");
        assert_eq!(company_blocker("Myla", CompanyNeeds::Deposit, CompanyWorld { deposit: true, ..OK }), None);
        let q = |seat| company_blocker("Quirrel", CompanyNeeds::View, CompanyWorld { seat, ..OK });
        assert!(q(SeatCheck::None).unwrap().contains("no window edge"));
        assert!(q(SeatCheck::Unsettled).unwrap().contains("settle"));
        assert!(q(SeatCheck::Low).unwrap().contains("lower half"));
        assert_eq!(q(SeatCheck::High), None);
        // A plain seat doesn't need the height.
        assert_eq!(company_blocker("Someone", CompanyNeeds::Seat, CompanyWorld { seat: SeatCheck::Low, ..OK }), None);
        // The user first: fullscreen apps and typing stop every calm scene.
        assert!(company_blocker("Cornifer", CompanyNeeds::Any, CompanyWorld { fullscreen: true, ..OK }).unwrap().ends_with("a fullscreen app is up"));
        assert!(company_blocker("Cornifer", CompanyNeeds::Any, CompanyWorld { typing: true, ..OK }).unwrap().ends_with("you're typing"));
    }
}
