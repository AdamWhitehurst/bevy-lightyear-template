use avian3d::prelude::Rotation;
use bevy::prelude::*;
use lightyear::prelude::*;
use protocol::map::MapInstanceId;
use protocol::world_object::{apply_object_components, WorldObjectDef, WorldObjectId};

/// Spawns a world object entity on the server.
///
/// Lightyear replicates it to all clients on the same map via the room system.
/// `MapInstanceId` triggers `on_map_instance_id_added`, which automatically adds
/// the entity to the correct Lightyear room.
///
/// All gameplay components (Position, RigidBody, CollisionLayers, ColliderConstructor,
/// ObjectCategory, VisualKind, etc.) come from the definition's reflected components.
pub fn spawn_world_object(
    commands: &mut Commands,
    id: WorldObjectId,
    def: &WorldObjectDef,
    map_id: MapInstanceId,
    registry: &AppTypeRegistry,
) -> Entity {
    let entity = commands
        .spawn((
            id,
            Rotation::default(),
            map_id,
            Replicate::to_clients(NetworkTarget::All),
        ))
        .id();

    let components = def
        .components
        .iter()
        .map(|c| {
            c.reflect_clone()
                .expect("world object component must be cloneable")
                .into_partial_reflect()
        })
        .collect();
    apply_object_components(commands, entity, components, registry.0.clone());
    entity
}
