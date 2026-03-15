use bevy::prelude::*;
use bevy::reflect::PartialReflect;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique identifier for a world object definition. Derived from the `.object.ron` filename.
///
/// Also used as a replicated ECS component — the single component Lightyear sends to clients
/// to identify which definition to look up in `WorldObjectDefRegistry`.
#[derive(Component, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct WorldObjectId(pub String);

/// Broad classification of world objects.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
#[reflect(Component, Serialize, Deserialize)]
pub enum ObjectCategory {
    Scenery,
    Interactive,
    ResourceNode,
    Item,
    Npc,
}

/// How the object is visually represented.
///
/// Visual assets are resolved lazily at spawn time via `asset_server.load`, following
/// the sprite rig cross-reference pattern. Deferred to the vox loading plan.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
#[reflect(Component, Serialize, Deserialize)]
pub enum VisualKind {
    /// Path to a .vox model relative to assets/.
    Vox(String),
    /// Path to a .rig.ron file.
    SpriteRig(String),
    /// Path to a sprite image.
    Sprite(String),
    /// No visual (server-only or invisible).
    None,
}

/// A loaded world object definition.
///
/// All fields are stored as type-erased reflect components. They are inserted via
/// `apply_object_components`, which uses `ReflectComponent::insert` on each.
#[derive(Asset, TypePath)]
pub struct WorldObjectDef {
    /// Reflect components deserialized from RON via `TypeRegistry`.
    /// Inserted on both server and client via `apply_object_components`.
    pub components: Vec<Box<dyn PartialReflect>>,
}

impl Clone for WorldObjectDef {
    fn clone(&self) -> Self {
        Self {
            components: self
                .components
                .iter()
                .map(|c| {
                    c.reflect_clone()
                        .expect("world object component must be cloneable")
                        .into_partial_reflect()
                })
                .collect(),
        }
    }
}

impl fmt::Debug for WorldObjectDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorldObjectDef")
            .field(
                "components",
                &self
                    .components
                    .iter()
                    .map(|c| c.reflect_type_path())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Error type for world object asset loading failures.
#[derive(Debug)]
pub enum WorldObjectLoadError {
    Io(std::io::Error),
    Ron(ron::error::SpannedError),
}

impl fmt::Display for WorldObjectLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Ron(e) => write!(f, "RON error: {e}"),
        }
    }
}

impl std::error::Error for WorldObjectLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Ron(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for WorldObjectLoadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ron::error::SpannedError> for WorldObjectLoadError {
    fn from(e: ron::error::SpannedError) -> Self {
        Self::Ron(e)
    }
}

impl From<ron::error::Error> for WorldObjectLoadError {
    fn from(e: ron::error::Error) -> Self {
        Self::Ron(ron::error::SpannedError {
            code: e,
            span: ron::error::Span {
                start: ron::error::Position { line: 0, col: 0 },
                end: ron::error::Position { line: 0, col: 0 },
            },
        })
    }
}
