use bevy::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

use super::types::TerrainDef;

/// All loaded terrain definitions, keyed by ID (e.g., "overworld", "homebase").
///
/// Populated during `AppState::Loading` via `TerrainPlugin` systems.
/// Available to both server and client after `AppState::Ready`.
#[derive(Resource, Clone, Debug)]
pub struct TerrainDefRegistry {
    pub terrains: HashMap<String, TerrainDef>,
}

impl TerrainDefRegistry {
    /// Looks up a terrain definition by ID.
    pub fn get(&self, id: &str) -> Option<&TerrainDef> {
        self.terrains.get(id)
    }
}

/// Lists terrain IDs for WASM builds (where `load_folder` is unavailable).
#[derive(Deserialize, Asset, TypePath)]
#[serde(transparent)]
pub struct TerrainManifest(pub Vec<String>);
