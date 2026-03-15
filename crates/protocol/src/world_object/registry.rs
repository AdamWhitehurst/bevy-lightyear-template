use bevy::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

use super::types::{WorldObjectDef, WorldObjectId};

/// All loaded world object definitions, keyed by ID.
///
/// Populated during `AppState::Loading` via `WorldObjectPlugin` systems.
/// Available to both server and client after `AppState::Ready`.
#[derive(Resource, Clone, Debug)]
pub struct WorldObjectDefRegistry {
    pub objects: HashMap<WorldObjectId, WorldObjectDef>,
}

impl WorldObjectDefRegistry {
    /// Looks up a world object definition by ID.
    pub fn get(&self, id: &WorldObjectId) -> Option<&WorldObjectDef> {
        self.objects.get(id)
    }
}

/// Lists object IDs for WASM builds (where `load_folder` is unavailable).
///
/// Must be updated manually when `.object.ron` files are added or removed —
/// the same convention as `abilities.manifest.ron`.
#[derive(Deserialize, Asset, TypePath)]
pub struct WorldObjectManifest(pub Vec<String>);
