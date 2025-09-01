use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimationConfig {
    pub groups: Vec<AnimationGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimationGroup {
    pub name: String,
    pub fps: f32,
    pub prefix: Option<String>,
    pub order: Option<Vec<String>>, // explicit frame names
    #[serde(default)]
    pub r#loop: bool,
}

impl AnimationConfig {
    pub fn load_str(s: &str) -> Result<Self> {
        Ok(serde_json::from_str::<AnimationConfig>(s)?)
    }

    pub fn get(&self, name: &str) -> Option<&AnimationGroup> {
        self.groups.iter().find(|g| g.name == name)
    }
}

// Simple time-based animator over a fixed-length frame list.
#[derive(Debug, Clone)]
pub struct Animator {
    fps: f32,
    len: usize,
    looping: bool,
    acc: f32,
    frame: usize,
}

impl Animator {
    pub fn new(fps: f32, len: usize, looping: bool) -> Self {
        Self { fps, len, looping, acc: 0.0, frame: 0 }
    }

    pub fn reset(&mut self) {
        self.acc = 0.0;
        self.frame = 0;
    }

    pub fn tick(&mut self, dt_seconds: f32) {
        if self.len == 0 || self.fps <= 0.0 {
            return;
        }
        self.acc += dt_seconds * self.fps;
        let mut steps = self.acc.floor() as i64;
        self.acc -= steps as f32;
        while steps > 0 {
            if self.frame + 1 < self.len {
                self.frame += 1;
            } else if self.looping {
                self.frame = 0;
            } else {
                // clamp at last frame
            }
            steps -= 1;
        }
    }

    pub fn frame_index(&self) -> usize { self.frame }

    pub fn set_frame(&mut self, index: usize) { self.frame = index.min(self.len.saturating_sub(1)); }

    pub fn len(&self) -> usize { self.len }
}
