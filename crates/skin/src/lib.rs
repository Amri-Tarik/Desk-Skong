use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ExportRoot {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub collection_name: String,
    pub texel_size: [f32; 2],
    pub sprites: Vec<ExportSprite>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSprite {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "UVRect")]
    pub uv_rect: UVRect,
    #[serde(rename = "PixelRect")]
    pub pixel_rect: PixelRect,
    #[serde(rename = "Bounds")]
    pub bounds: BoundsData,
    #[serde(rename = "UntrimmedBounds")]
    pub untrimmed_bounds: BoundsData,
    #[serde(rename = "Pivot")]
    pub pivot: [f32; 2],
    #[serde(rename = "Indices")]
    pub indices: Option<Vec<i32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UVRect {
    #[serde(rename = "UMin")]
    pub u_min: f32,
    #[serde(rename = "VMin")]
    pub v_min: f32,
    #[serde(rename = "UMax")]
    pub u_max: f32,
    #[serde(rename = "VMax")]
    pub v_max: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PixelRect {
    #[serde(rename = "X")]
    pub x: i32,
    #[serde(rename = "Y")]
    pub y: i32,
    #[serde(rename = "W")]
    pub w: i32,
    #[serde(rename = "H")]
    pub h: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundsData {
    #[serde(rename = "Center")]
    pub center: [f32; 3],
    #[serde(rename = "Extents")]
    pub extents: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct Skin {
    pub data: ExportRoot,
    name_to_index: HashMap<String, usize>,
}

impl Skin {
    pub fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("reading skin JSON: {}", path.as_ref().display()))?;
        let data: ExportRoot = serde_json::from_str(&text)
            .with_context(|| format!("parsing skin JSON: {}", path.as_ref().display()))?;
        let mut name_to_index = HashMap::with_capacity(data.sprites.len());
        for (i, s) in data.sprites.iter().enumerate() {
            name_to_index.insert(s.name.clone(), i);
        }
        Ok(Skin { data, name_to_index })
    }

    pub fn atlas_size(&self) -> (u32, u32) {
        (self.data.atlas_width, self.data.atlas_height)
    }

    pub fn sprite_index(&self, name: &str) -> Option<usize> {
        self.name_to_index.get(name).copied()
    }

    pub fn sprite_name(&self, index: usize) -> Option<&str> {
        self.data.sprites.get(index).map(|s| s.name.as_str())
    }

    pub fn sprite_count(&self) -> usize {
        self.data.sprites.len()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.data.sprites.iter().map(|s| s.name.as_str())
    }

    pub fn require_index(&self, name: &str) -> Result<usize> {
        self.sprite_index(name)
            .with_context(|| format!("sprite not found: {name}"))
    }
}
