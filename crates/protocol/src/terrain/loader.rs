use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::reflect::{TypePath, TypeRegistryArc};

use super::types::TerrainDef;
use crate::reflect_loader;

/// Custom asset loader for `.terrain.ron` files using `TypeRegistry` for
/// reflect-based component deserialization.
#[derive(TypePath)]
pub(super) struct TerrainDefLoader {
    type_registry: TypeRegistryArc,
}

impl FromWorld for TerrainDefLoader {
    fn from_world(world: &mut World) -> Self {
        Self {
            type_registry: world.resource::<AppTypeRegistry>().0.clone(),
        }
    }
}

impl AssetLoader for TerrainDefLoader {
    type Asset = TerrainDef;
    type Settings = ();
    type Error = reflect_loader::ReflectLoadError;

    fn extensions(&self) -> &[&str] {
        &["terrain.ron"]
    }

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let registry = self.type_registry.read();
        let components = reflect_loader::deserialize_component_map(&bytes, &registry)?;
        Ok(TerrainDef { components })
    }
}
