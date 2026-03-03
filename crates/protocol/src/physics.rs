use avian3d::prelude::*;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::map::MapInstanceId;

/// Physics collision hooks for map instance isolation.
/// Only one CollisionHooks impl is allowed per app -- future hooks
/// (one-way platforms, conveyors) must be added to this struct.
#[derive(SystemParam)]
pub struct MapCollisionHooks<'w, 's> {
    map_ids: Query<'w, 's, &'static MapInstanceId>,
}

impl CollisionHooks for MapCollisionHooks<'_, '_> {
    fn filter_pairs(&self, entity1: Entity, entity2: Entity, _commands: &mut Commands) -> bool {
        match (self.map_ids.get(entity1), self.map_ids.get(entity2)) {
            (Ok(a), Ok(b)) => a.0 == b.0,
            _ => true,
        }
    }
}
