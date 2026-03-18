use std::collections::HashMap;

use avian3d::prelude::ActiveCollisionHooks;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Identifies which map instance an entity belongs to.
/// Semantic enum — safe to replicate, no Entity references.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, Reflect)]
#[type_path = "protocol::map"]
#[require(ActiveCollisionHooks::FILTER_PAIRS)]
pub enum MapInstanceId {
    Overworld,
    Homebase { owner: u64 },
}

/// Maps semantic `MapInstanceId` to local `VoxelMapInstance` entities.
/// Each side (server/client) maintains independently.
#[derive(Resource, Default)]
pub struct MapRegistry(pub HashMap<MapInstanceId, Entity>);

impl MapRegistry {
    pub fn get(&self, id: &MapInstanceId) -> Entity {
        *self
            .0
            .get(id)
            .unwrap_or_else(|| panic!("MapRegistry lookup failed for {id:?} — map not registered"))
    }

    pub fn insert(&mut self, id: MapInstanceId, entity: Entity) {
        self.0.insert(id, entity);
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
#[type_path = "protocol::map"]
pub enum MapSwitchTarget {
    Overworld,
    Homebase,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_instance_id_equality() {
        assert_eq!(MapInstanceId::Overworld, MapInstanceId::Overworld);
        assert_ne!(
            MapInstanceId::Overworld,
            MapInstanceId::Homebase { owner: 0 }
        );
    }

    #[test]
    fn map_registry_get_panics_on_missing() {
        let registry = MapRegistry::default();
        let result = std::panic::catch_unwind(|| registry.get(&MapInstanceId::Overworld));
        assert!(result.is_err());
    }

    #[test]
    fn map_registry_insert_and_get() {
        let mut registry = MapRegistry::default();
        let entity = Entity::from_bits(42);
        registry.insert(MapInstanceId::Overworld, entity);
        assert_eq!(registry.get(&MapInstanceId::Overworld), entity);
    }
}
