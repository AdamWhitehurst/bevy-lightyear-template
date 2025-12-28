use bevy::prelude::*;
use bevy_voxel_world::prelude::*;
use leafwing_input_manager::prelude::*;
use lightyear::prelude::{Controlled, MessageReceiver, MessageSender};
use protocol::{
    MapWorld, PlayerActions, VoxelChannel, VoxelEditBroadcast, VoxelEditRequest, VoxelStateSync,
    VoxelType,
};

/// Plugin managing client-side voxel map functionality
pub struct ClientMapPlugin;

impl Plugin for ClientMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelWorldPlugin::<MapWorld>::with_config(MapWorld))
            .add_systems(
                Update,
                (
                    handle_voxel_broadcasts,
                    handle_state_sync,
                    protocol::attach_chunk_colliders,
                    handle_voxel_input,
                ),
            );
    }
}

fn handle_voxel_broadcasts(
    mut receiver: Query<&mut MessageReceiver<VoxelEditBroadcast>>,
    mut voxel_world: VoxelWorld<MapWorld>,
) {
    for mut message_receiver in receiver.iter_mut() {
        message_receiver.receive().for_each(|broadcast| {
            voxel_world.set_voxel(broadcast.position, broadcast.voxel.into());
        });
    }
}

fn handle_state_sync(
    mut receiver: Query<&mut MessageReceiver<VoxelStateSync>>,
    mut voxel_world: VoxelWorld<MapWorld>,
) {
    for mut message_receiver in receiver.iter_mut() {
        message_receiver.receive().for_each(|sync| {
            for (position, voxel) in &sync.modifications {
                voxel_world.set_voxel(*position, (*voxel).into());
            }
        });
    }
}

/// System for sending edit requests (called from gameplay systems)
pub fn send_voxel_edit(
    position: IVec3,
    voxel: VoxelType,
    mut message_writer: MessageWriter<VoxelEditRequest>,
) {
    message_writer.write(VoxelEditRequest { position, voxel });
}

/// Handle voxel editing input from mouse
fn handle_voxel_input(
    voxel_world: VoxelWorld<MapWorld>,
    action_state: Query<&ActionState<PlayerActions>, With<Controlled>>,
    camera: Query<(&Camera, &GlobalTransform), With<VoxelWorldCamera<MapWorld>>>,
    windows: Query<&Window>,
    mut sender: Query<&mut MessageSender<VoxelEditRequest>>,
) {
    let Ok(action) = action_state.single() else {
        return;
    };
    let Ok((camera, transform)) = camera.single() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };

    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(transform, cursor) else {
        return;
    };

    let place = action.just_pressed(&PlayerActions::PlaceVoxel);
    let remove = action.just_pressed(&PlayerActions::RemoveVoxel);

    if !place && !remove {
        return;
    }

    if let Some(hit) = voxel_world.raycast(ray, &|_| true) {
        let request = if remove {
            VoxelEditRequest {
                position: hit.voxel_pos(),
                voxel: VoxelType::Air,
            }
        } else {
            // Place adjacent to hit surface
            let normal = hit.normal.unwrap_or(Vec3::Y);
            let place_pos = hit.voxel_pos() + normal.as_ivec3();
            VoxelEditRequest {
                position: place_pos,
                voxel: VoxelType::Solid(0),
            }
        };

        for mut message_sender in sender.iter_mut() {
            message_sender.send::<VoxelChannel>(request.clone());
        }
    }
}
