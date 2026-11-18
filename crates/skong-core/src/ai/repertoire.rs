//! The acts the brain knows about (a lightweight view of [`ActLibrary`], constructible
//! without clip data for tests) and mood-weighted picking with cooldowns.
use std::collections::BTreeMap;

use crate::acts::{ActId, ActLibrary, Requirement};

use super::needs::Mind;
use super::rng::Rng;

/// What the brain needs to know about one act.
#[derive(Clone, Debug, PartialEq)]
pub struct ActInfo {
    pub id: ActId,
    pub name: String,
    pub tags: Vec<String>,
    /// Base pick weight; 0 = only started explicitly by name.
    pub weight: f32,
    pub cooldown: f32,
    pub requires: Requirement,
}

impl ActInfo {
    /// Can this act start in the knight's current situation?
    /// `at_bench` = a seat is available here (at a bench, or able to perch on this ledge).
    pub fn startable(&self, grounded: bool, at_bench: bool) -> bool {
        match self.requires {
            Requirement::Any => true,
            Requirement::Grounded => grounded,
            Requirement::Seat => grounded && at_bench,
        }
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

/// All acts, indexed by name.
#[derive(Clone, Debug, Default)]
pub struct ActCatalog {
    acts: Vec<ActInfo>,
}

impl ActCatalog {
    pub fn new(acts: Vec<ActInfo>) -> Self {
        Self { acts }
    }

    pub fn from_library(lib: &ActLibrary) -> Self {
        Self::new(
            lib.iter()
                .map(|(id, a)| ActInfo { id, name: a.name.clone(), tags: a.tags.clone(), weight: a.weight, cooldown: a.cooldown, requires: a.requires })
                .collect(),
        )
    }

    pub fn id(&self, name: &str) -> Option<ActId> {
        self.acts.iter().find(|a| a.name == name).map(|a| a.id)
    }

    pub fn get(&self, id: ActId) -> Option<&ActInfo> {
        self.acts.iter().find(|a| a.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ActInfo> {
        self.acts.iter()
    }
}

/// Picks acts by mood and remembers cooldowns (brain clock time when usable again).
#[derive(Clone, Debug, Default)]
pub(crate) struct Repertoire {
    pub catalog: ActCatalog,
    /// Keyed by `ActId.0` (ActId is not `Ord`).
    ready_at: BTreeMap<u16, f32>,
}

impl Repertoire {
    pub fn new(catalog: ActCatalog) -> Self {
        Self { catalog, ready_at: BTreeMap::new() }
    }

    pub fn ready(&self, id: ActId, now: f32) -> bool {
        self.ready_at.get(&id.0).is_none_or(|&t| t <= now)
    }

    pub fn used(&mut self, id: ActId, now: f32) {
        let cd = self.catalog.get(id).map_or(0.0, |a| a.cooldown);
        self.ready_at.insert(id.0, now + cd);
    }

    /// Weighted pick among acts whose tags fit the mood. `prefer` multiplies acts that
    /// carry that tag (e.g. "curious" when arriving somewhere new).
    pub fn pick(&self, mind: &Mind, user_away: bool, (grounded, at_bench): (bool, bool), prefer: Option<&str>, now: f32, rng: &mut Rng) -> Option<ActId> {
        let candidates: Vec<(ActId, f32)> = self
            .catalog
            .iter()
            .filter(|a| {
                a.weight > 0.0
                    && !a.has_tag("travel") && !a.has_tag("intro")
                    && a.startable(grounded, at_bench)
                    && self.ready(a.id, now)
            })
            .map(|a| {
                let fit = a.tags.iter().map(|t| mind.tag_weight(t, user_away)).fold(0.0, f32::max);
                let bonus = match prefer {
                    Some(tag) if a.has_tag(tag) => 3.0,
                    _ => 1.0,
                };
                (a.id, a.weight * fit * fit * bonus)
            })
            .collect();
        rng.pick_weighted(&candidates)
    }
}
