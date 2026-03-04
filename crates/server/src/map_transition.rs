use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::*;
use protocol::map::{
    MapChannel, MapInstanceId, MapSwitchTarget, MapTransitionStart, PlayerMapSwitchRequest,
};
use protocol::{CharacterMarker, OverworldMap};
use voxel_map_engine::prelude::{flat_terrain_voxels, ChunkTarget, Homebase, VoxelMapInstance};

pub struct ServerMapTransitionPlugin;

impl Plugin for ServerMapTransitionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (handle_map_switch_requests, tick_map_transition_timers),
        );
    }
}

/// Marker for entities in active map transition, with a timer.
#[derive(Component)]
pub struct MapTransitionTimer(pub Timer);

fn handle_map_switch_requests(
    mut commands: Commands,
    mut receiver: Query<(Entity, &mut MessageReceiver<PlayerMapSwitchRequest>), With<ClientOf>>,
    mut sender: Query<&mut MessageSender<MapTransitionStart>, With<ClientOf>>,
    overworld: Res<OverworldMap>,
    homebases: Query<(Entity, &Homebase)>,
    players: Query<(Entity, &ControlledBy), With<CharacterMarker>>,
) {
    for (client_entity, mut message_receiver) in receiver.iter_mut() {
        for request in message_receiver.receive() {
            let Some((player_entity, _)) =
                players.iter().find(|(_, ctrl)| ctrl.owner == client_entity)
            else {
                warn!(
                    "Map switch request from client {client_entity:?} but no owned character found"
                );
                continue;
            };

            let target_map = match request.target {
                MapSwitchTarget::Overworld => overworld.0,
                MapSwitchTarget::Homebase => {
                    find_or_spawn_homebase(&mut commands, player_entity, &homebases)
                }
            };

            initiate_map_transition(&mut commands, player_entity, target_map);

            if let Ok(mut msg_sender) = sender.get_mut(client_entity) {
                msg_sender.send::<MapChannel>(MapTransitionStart {
                    target: request.target,
                });
            }
        }
    }
}

/// Find an existing homebase for a player, or spawn a new one.
pub fn find_or_spawn_homebase(
    commands: &mut Commands,
    player_entity: Entity,
    homebases: &Query<(Entity, &Homebase)>,
) -> Entity {
    if let Some((map_entity, _)) = homebases.iter().find(|(_, hb)| hb.owner == player_entity) {
        return map_entity;
    }

    let (instance, config, marker) = VoxelMapInstance::homebase(
        player_entity,
        IVec3::new(8, 4, 8),
        Arc::new(flat_terrain_voxels),
    );
    commands
        .spawn((instance, config, marker, Transform::default()))
        .id()
}

/// Execute map transition: pause physics, update map association, teleport.
/// Used by both client-requested and server-initiated transitions.
pub fn initiate_map_transition(commands: &mut Commands, player_entity: Entity, target_map: Entity) {
    commands.entity(player_entity).insert((
        RigidBodyDisabled,
        DisableRollback,
        MapInstanceId(target_map),
        ChunkTarget::new(target_map, 4),
        Position(Vec3::new(0.0, 30.0, 0.0)),
        LinearVelocity(Vec3::ZERO),
        MapTransitionTimer(Timer::from_seconds(5.0, TimerMode::Once)),
    ));
}

pub fn tick_map_transition_timers(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut MapTransitionTimer)>,
) {
    for (entity, mut timer) in &mut query {
        timer.0.tick(time.delta());
        if timer.0.is_finished() {
            commands
                .entity(entity)
                .remove::<(RigidBodyDisabled, DisableRollback, MapTransitionTimer)>();
        }
    }
}
