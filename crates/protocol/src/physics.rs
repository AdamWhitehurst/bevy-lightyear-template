use avian3d::prelude::*;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::map::MapInstanceId;

/// Collision hooks for map instance isolation.
/// Only one `CollisionHooks` impl per app — extend this struct for future needs.
#[derive(SystemParam)]
pub struct MapCollisionHooks<'w, 's> {
    map_ids: Query<'w, 's, &'static MapInstanceId>,
}

impl CollisionHooks for MapCollisionHooks<'_, '_> {
    fn filter_pairs(&self, entity1: Entity, entity2: Entity, _commands: &mut Commands) -> bool {
        let entity1_id = self.map_ids.get(entity1).ok();
        let entity2_id = self.map_ids.get(entity2).ok();
        match (entity1_id, entity2_id) {
            (Some(a), Some(b)) => a == b,
            _ => {
                warn!("Entity missing MapInstanceId. Entity {entity1:?}: {entity1_id:?}. Entity {entity2:?}: {entity2_id:?}");
                false
            }
        }
    }
}
