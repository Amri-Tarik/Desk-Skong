//! Hidden achievements for the panel's Achievements tab. Sent as
//! [`crate::PetMsg::Achievements`] when a panel connects and whenever something unlocks
//! (never in the ~4 Hz status). Locked entries carry no title or description at all:
//! the pet doesn't tell, so no panel can spoil them.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AchievementsInfo {
    /// Every achievement, in definition order (locked ones too, as blanks).
    pub entries: Vec<AchievementEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AchievementEntry {
    /// A vague category ("a visitor", "the cursor"), shown while locked.
    pub hint: Option<String>,
    /// Filled once unlocked.
    pub unlocked: Option<UnlockedAchievement>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UnlockedAchievement {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Unix seconds.
    pub at: u64,
}

impl AchievementsInfo {
    pub fn total(&self) -> usize {
        self.entries.len()
    }

    pub fn unlocked(&self) -> usize {
        self.entries.iter().filter(|e| e.unlocked.is_some()).count()
    }
}
