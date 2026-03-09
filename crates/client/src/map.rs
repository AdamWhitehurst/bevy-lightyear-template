use std::sync::Arc;

use avian3d::prelude::{ColliderDisabled, LinearVelocity, Position, RigidBodyDisabled};
use bevy::{prelude::*, window::PrimaryWindow};
use leafwing_input_manager::prelude::*;
use lightyear::prelude::{Controlled, DisableRollback, MessageReceiver, MessageSender, Predicted};
use protocol::map::{MapChannel, MapTransitionEnd, MapTransitionReady, MapTransitionStart};
use protocol::{
    CharacterMarker, MapInstanceId, MapRegistry, MapWorld, PendingTransition, PlayerActions,
    TransitionReadySent, VoxelChannel, VoxelEditBroadcast, VoxelEditRequest, VoxelStateSync,
    VoxelType,
};
use ui::MapTransitionState;
use voxel_map_engine::prelude::{
    flat_terrain_voxels, ChunkTarget, PendingChunks, VoxelMapConfig, VoxelMapInstance, VoxelPlugin,
    VoxelWorld, WorldVoxel,
};

const RAYCAST_MAX_DISTANCE: f32 = 100.0;

/// Plugin managing client-side voxel map functionality.
pub struct ClientMapPlugin;

impl Plugin for ClientMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelPlugin)
            .init_resource::<MapWorld>()
            .init_resource::<MapRegistry>()
            .add_systems(Startup, spawn_overworld)
            .add_systems(
                Update,
                (
                    attach_chunk_target_to_player,
                    handle_voxel_broadcasts,
                    handle_state_sync,
                    protocol::attach_chunk_colliders,
                ),
            )
            .add_systems(
                PostUpdate,
                handle_voxel_input.after(TransformSystems::Propagate),
            )
            .add_systems(Update, handle_map_transition_start)
            .add_systems(
                Update,
                (check_transition_chunks_loaded, handle_map_transition_end)
                    .run_if(in_state(MapTransitionState::Transitioning)),
            );
    }
}

/// Resource tracking the primary overworld map entity.
#[derive(Resource)]
pub struct OverworldMap(pub Entity);

fn spawn_overworld(
    mut commands: Commands,
    map_world: Res<MapWorld>,
    mut registry: ResMut<MapRegistry>,
) {
    let map = commands
        .spawn((
            VoxelMapInstance::new(5),
            VoxelMapConfig::new(map_world.seed, 2, None, 5, Arc::new(flat_terrain_voxels)),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();
    commands.insert_resource(OverworldMap(map));
    registry.insert(MapInstanceId::Overworld, map);
}

fn attach_chunk_target_to_player(
    mut commands: Commands,
    registry: Res<MapRegistry>,
    players: Query<
        (Entity, &MapInstanceId),
        (With<Predicted>, With<CharacterMarker>, Without<ChunkTarget>),
    >,
) {
    for (entity, map_id) in &players {
        let map_entity = registry.get(map_id);
        commands
            .entity(entity)
            .insert(ChunkTarget::new(map_entity, 4));
    }
}

fn handle_voxel_broadcasts(
    mut receiver: Query<&mut MessageReceiver<VoxelEditBroadcast>>,
    overworld: Res<OverworldMap>,
    mut voxel_world: VoxelWorld,
) {
    for mut message_receiver in receiver.iter_mut() {
        for broadcast in message_receiver.receive() {
            voxel_world.set_voxel(
                overworld.0,
                broadcast.position,
                WorldVoxel::from(broadcast.voxel),
            );
        }
    }
}

fn handle_state_sync(
    mut receiver: Query<&mut MessageReceiver<VoxelStateSync>>,
    overworld: Res<OverworldMap>,
    mut voxel_world: VoxelWorld,
) {
    for mut message_receiver in receiver.iter_mut() {
        for sync in message_receiver.receive() {
            for &(pos, voxel_type) in &sync.modifications {
                voxel_world.set_voxel(overworld.0, pos, WorldVoxel::from(voxel_type));
            }
        }
    }
}

fn handle_voxel_input(
    overworld: Res<OverworldMap>,
    voxel_world: VoxelWorld,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    action_query: Query<&ActionState<PlayerActions>, With<Controlled>>,
    message_sender: Query<&mut MessageSender<VoxelEditRequest>>,
) {
    let Ok(action_state) = action_query.single() else {
        return;
    };

    let removing = action_state.just_pressed(&PlayerActions::RemoveVoxel);
    let placing = action_state.just_pressed(&PlayerActions::PlaceVoxel);
    if !removing && !placing {
        return;
    }

    let Some(ray) = camera_ray(&camera_query, &window_query) else {
        return;
    };

    let Some(hit) = voxel_world.raycast(overworld.0, ray, RAYCAST_MAX_DISTANCE, |v| {
        matches!(v, WorldVoxel::Solid(_))
    }) else {
        return;
    };

    if removing {
        send_voxel_edit(hit.position, VoxelType::Air, message_sender);
    } else if let Some(normal) = hit.normal {
        let place_pos = hit.position + normal.as_ivec3();
        send_voxel_edit(place_pos, VoxelType::Solid(0), message_sender);
    }
}

fn camera_ray(
    camera_query: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: &Query<&Window, With<PrimaryWindow>>,
) -> Option<Ray3d> {
    let (camera, camera_transform) = camera_query.single().ok()?;
    let window = window_query.single().ok()?;
    let cursor_pos = window.cursor_position()?;
    if let Some(rect) = camera.logical_viewport_rect() {
        info!("rect.min = {:?}, cursor_pos = {:?}", rect.min, cursor_pos);
    }
    let viewport_pos = if let Some(rect) = camera.logical_viewport_rect() {
        // rect.min is 0,0 on primary
        cursor_pos - rect.min
    } else {
        cursor_pos
    };

    camera
        .viewport_to_world(camera_transform, viewport_pos)
        .ok()
}

/// Send a voxel edit request to the server.
pub fn send_voxel_edit(
    position: IVec3,
    voxel: VoxelType,
    mut message_sender: Query<&mut MessageSender<VoxelEditRequest>>,
) {
    for mut sender in message_sender.iter_mut() {
        debug!("Sending voxel edit request to server: {:?}", position);
        sender.send::<VoxelChannel>(VoxelEditRequest { position, voxel });
    }
}

pub fn handle_map_transition_start(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<MapTransitionStart>>,
    mut registry: ResMut<MapRegistry>,
    player_query: Query<Entity, (With<Predicted>, With<CharacterMarker>, With<Controlled>)>,
) {
    for mut receiver in &mut receivers {
        for transition in receiver.receive() {
            info!("Received MapTransitionStart for {:?}", transition.target);

            let player = player_query
                .single()
                .expect("Predicted player must exist when receiving MapTransitionStart");

            // Freeze player, disable rollback, teleport to spawn position
            commands.entity(player).insert((
                RigidBodyDisabled,
                ColliderDisabled,
                DisableRollback,
                PendingTransition(transition.target.clone()),
                Position(transition.spawn_position),
                LinearVelocity(Vec3::ZERO),
            ));

            if !registry.0.contains_key(&transition.target) {
                let generator = generator_for_map(&transition.target);
                let map_entity = spawn_map_instance(
                    &mut commands,
                    &transition.target,
                    transition.seed,
                    transition.bounds,
                    generator,
                );
                registry.insert(transition.target.clone(), map_entity);
            }

            let map_entity = registry.get(&transition.target);
            commands
                .entity(player)
                .insert(ChunkTarget::new(map_entity, 4));
        }
    }
}

fn generator_for_map(
    map_id: &MapInstanceId,
) -> Arc<dyn Fn(IVec3) -> Vec<WorldVoxel> + Send + Sync> {
    match map_id {
        MapInstanceId::Overworld => Arc::new(flat_terrain_voxels),
        MapInstanceId::Homebase { .. } => Arc::new(flat_terrain_voxels),
    }
}

fn spawn_map_instance(
    commands: &mut Commands,
    map_id: &MapInstanceId,
    seed: u64,
    bounds: Option<IVec3>,
    generator: Arc<dyn Fn(IVec3) -> Vec<WorldVoxel> + Send + Sync>,
) -> Entity {
    let tree_height = match map_id {
        MapInstanceId::Overworld => 5,
        MapInstanceId::Homebase { .. } => 3,
    };
    let spawning_distance = bounds.map(|b| b.max_element().max(1) as u32).unwrap_or(10);

    let entity = commands
        .spawn((
            VoxelMapInstance::new(tree_height),
            VoxelMapConfig::new(seed, spawning_distance, bounds, tree_height, generator),
            Transform::default(),
            map_id.clone(),
        ))
        .id();

    info!("Spawned client map instance for {map_id:?}: {entity:?}");
    entity
}

pub fn check_transition_chunks_loaded(
    mut commands: Commands,
    player_query: Query<
        (Entity, &PendingTransition),
        (
            With<Predicted>,
            With<CharacterMarker>,
            Without<TransitionReadySent>,
        ),
    >,
    registry: Res<MapRegistry>,
    maps: Query<(&VoxelMapInstance, Option<&PendingChunks>)>,
    mut senders: Query<&mut MessageSender<MapTransitionReady>>,
) {
    let Ok((player, pending)) = player_query.single() else {
        return;
    };
    let map_entity = registry.get(&pending.0);
    let (map, pending_chunks) = maps
        .get(map_entity)
        .expect("Pending transition map must exist in ECS");

    let Some(pending_chunks) = pending_chunks else {
        return;
    };

    if map.loaded_chunks.is_empty()
        || !pending_chunks.tasks.is_empty()
        || !pending_chunks.pending_positions.is_empty()
    {
        return;
    }

    info!(
        "Transition chunks loaded for {:?}, sending ready to server",
        pending.0
    );

    commands.entity(player).insert(TransitionReadySent);

    for mut sender in &mut senders {
        sender.send::<MapChannel>(MapTransitionReady);
    }
}

pub fn handle_map_transition_end(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<MapTransitionEnd>>,
    player_query: Query<
        Entity,
        (
            With<Predicted>,
            With<CharacterMarker>,
            With<PendingTransition>,
        ),
    >,
    mut next_transition: ResMut<NextState<MapTransitionState>>,
) {
    for mut receiver in &mut receivers {
        for _end in receiver.receive() {
            info!("Received MapTransitionEnd, resuming play");

            let Ok(player) = player_query.single() else {
                warn!("Received MapTransitionEnd but no transitioning player");
                continue;
            };

            commands.entity(player).remove::<(
                RigidBodyDisabled,
                ColliderDisabled,
                DisableRollback,
                PendingTransition,
                TransitionReadySent,
            )>();

            next_transition.set(MapTransitionState::Playing);
        }
    }
}
