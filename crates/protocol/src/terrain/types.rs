use bevy::prelude::*;
use bevy::reflect::PartialReflect;
use std::fmt;

/// A loaded terrain definition. Component map loaded from `.terrain.ron`.
#[derive(Asset, TypePath)]
pub struct TerrainDef {
    /// Reflect components deserialized from RON via `TypeRegistry`.
    /// Applied to map entities via `apply_object_components`.
    pub components: Vec<Box<dyn PartialReflect>>,
}

impl Clone for TerrainDef {
    fn clone(&self) -> Self {
        Self {
            components: self
                .components
                .iter()
                .map(|c| {
                    c.reflect_clone()
                        .expect("terrain component must be cloneable")
                        .into_partial_reflect()
                })
                .collect(),
        }
    }
}

impl fmt::Debug for TerrainDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerrainDef")
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
