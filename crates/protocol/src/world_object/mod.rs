mod loader;
mod plugin;
mod registry;
mod spawn;
mod types;

pub mod loading;

pub use loader::deserialize_world_object;
pub use plugin::WorldObjectPlugin;
pub use registry::{WorldObjectDefRegistry, WorldObjectManifest};
pub use spawn::apply_object_components;
pub use types::{
    ActiveTransformation, DeathEffect, ObjectCategory, OnDeathEffects, PlacementOffset,
    ReflectPersist, ReflectSpawnOnly, VisualKind, WorldObjectDef, WorldObjectId,
    WorldObjectLoadError,
};
