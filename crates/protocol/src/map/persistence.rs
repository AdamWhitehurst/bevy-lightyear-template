use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Marker: this entity should be saved with its map.
#[derive(Component, Clone, Debug, Default)]
pub struct MapSaveTarget;

/// Identifies the type of a saved entity for reconstruction.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SavedEntityKind {
    RespawnPoint,
}

/// A single entity serialized for persistence.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SavedEntity {
    pub kind: SavedEntityKind,
    pub position: Vec3,
}
