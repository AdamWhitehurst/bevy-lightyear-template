use std::sync::Arc;

use bevy::{prelude::*, window::PrimaryWindow};
use leafwing_input_manager::prelude::*;
use lightyear::prelude::{Controlled, MessageReceiver, MessageSender};
use protocol::{
    MapWorld, PlayerActions, VoxelChannel, VoxelEditBroadcast, VoxelEditRequest, VoxelStateSync,
    VoxelType,
};
use voxel_map_engine::prelude::{
    flat_terrain_voxels, ChunkTarget, VoxelMapConfig, VoxelMapInstance, VoxelPlugin, VoxelWorld,
    WorldVoxel,
};

const RAYCAST_MAX_DISTANCE: f32 = 100.0;

/// Plugin managing client-side voxel map functionality.
pub struct ClientMapPlugin;

impl Plugin for ClientMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelPlugin)
            .init_resource::<MapWorld>()
            .add_systems(Startup, spawn_overworld)
            .add_systems(
                Update,
                (
                    attach_chunk_target_to_camera,
                    handle_voxel_broadcasts,
                    handle_state_sync,
                    protocol::attach_chunk_colliders,
                ),
            )
            .add_systems(
                PostUpdate,
                handle_voxel_input.after(TransformSystems::Propagate),
            );
    }
}

use protocol::OverworldMap;

fn spawn_overworld(mut commands: Commands, map_world: Res<MapWorld>) {
    let map = commands
        .spawn((
            VoxelMapInstance::new(5),
            VoxelMapConfig::new(map_world.seed, 2, None, 5, Arc::new(flat_terrain_voxels)),
            Transform::default(),
        ))
        .id();
    commands.insert_resource(OverworldMap(map));
}

fn attach_chunk_target_to_camera(
    mut commands: Commands,
    overworld: Res<OverworldMap>,
    cameras: Query<Entity, (With<Camera3d>, Without<ChunkTarget>)>,
) {
    for entity in &cameras {
        commands
            .entity(entity)
            .insert(ChunkTarget::new(overworld.0, 4));
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
