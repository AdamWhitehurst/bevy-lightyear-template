use bevy::prelude::*;
use lightyear::prelude::{RoomEvent, RoomTarget};

use crate::map::MapInstanceId;

/// Remove an entity from its current room, update MapInstanceId.
/// Optionally update Position. Does NOT call AddEntity.
pub fn relocate_remove(
    commands: &mut Commands,
    entity: Entity,
    old_room: Entity,
    target_map_id: &MapInstanceId,
    spawn_position: Option<Vec3>,
) {
    commands.trigger(RoomEvent {
        room: old_room,
        target: RoomTarget::RemoveEntity(entity),
    });
    commands.entity(entity).insert(target_map_id.clone());
    if let Some(pos) = spawn_position {
        commands
            .entity(entity)
            .insert(avian3d::prelude::Position(pos));
    }
}

/// Add an entity to a room. Counterpart to relocate_remove.
pub fn relocate_add(commands: &mut Commands, entity: Entity, new_room: Entity) {
    commands.trigger(RoomEvent {
        room: new_room,
        target: RoomTarget::AddEntity(entity),
    });
}
