use bevy::prelude::*;
use bevy::reflect::PartialReflect;
use std::fmt;
use voxel_map_engine::config::MapDimensions;

/// A loaded terrain definition. Component map loaded from `.terrain.ron`.
#[derive(Asset, TypePath)]
pub struct TerrainDef {
    /// Reflect components deserialized from RON via `TypeRegistry`.
    /// Applied to map entities via `apply_object_components`.
    pub components: Vec<Box<dyn PartialReflect>>,
}

impl TerrainDef {
    /// Finds and clones the `MapDimensions` component from this def.
    pub fn map_dimensions(&self) -> Option<MapDimensions> {
        self.components
            .iter()
            .find_map(|c| c.try_downcast_ref::<MapDimensions>().cloned())
    }
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
