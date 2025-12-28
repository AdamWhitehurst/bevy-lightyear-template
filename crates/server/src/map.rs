use bevy::prelude::*;
use bevy_voxel_world::prelude::*;
use lightyear::prelude::{
    Connected, MessageReceiver, MessageSender, NetworkTarget, Server, ServerMultiMessageSender,
};
use protocol::{
    MapWorld, VoxelChannel, VoxelEditBroadcast, VoxelEditRequest, VoxelStateSync, VoxelType,
};

/// Plugin managing server-side voxel map functionality
pub struct ServerMapPlugin;

impl Plugin for ServerMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelWorldPlugin::<MapWorld>::with_config(MapWorld))
            .init_resource::<VoxelModifications>()
            .add_systems(Startup, spawn_voxel_camera)
            .add_systems(Update, (handle_voxel_edit_requests, protocol::attach_chunk_colliders))
            .add_observer(send_initial_voxel_state);
    }
}

/// Spawn a dummy camera for voxel world chunk management (server doesn't render)
fn spawn_voxel_camera(mut commands: Commands) {
    commands.spawn((
        Camera::default(),
        Transform::from_xyz(0.0, 10.0, 0.0),
        GlobalTransform::default(),
        VoxelWorldCamera::<MapWorld>::default(),
    ));
}

/// Tracks all voxel modifications for state sync
#[derive(Resource, Default)]
struct VoxelModifications {
    modifications: Vec<(IVec3, VoxelType)>,
}

fn handle_voxel_edit_requests(
    mut receiver: Query<&mut MessageReceiver<VoxelEditRequest>>,
    mut sender: ServerMultiMessageSender,
    server: Single<&Server>,
    mut modifications: ResMut<VoxelModifications>,
    mut voxel_world: VoxelWorld<MapWorld>,
) {
    let server_ref = server.into_inner();
    for mut message_receiver in receiver.iter_mut() {
        for request in message_receiver.receive() {
            eprintln!("✓ Server received and processing voxel edit: {:?}", request);

            // TODO: Add admin permission check here

            // Apply voxel change
            voxel_world.set_voxel(request.position, request.voxel.into());

            // Track modification
            modifications
                .modifications
                .push((request.position, request.voxel));

            // Broadcast to all clients
            sender
                .send::<_, VoxelChannel>(
                    &VoxelEditBroadcast {
                        position: request.position,
                        voxel: request.voxel,
                    },
                    server_ref,
                    &NetworkTarget::All,
                )
                .ok();
        }
    }
}

/// System to send initial state to newly connected clients
fn send_initial_voxel_state(
    trigger: On<Add, Connected>,
    modifications: Res<VoxelModifications>,
    mut sender: Query<&mut MessageSender<VoxelStateSync>>,
) {
    let Ok(mut message_sender) = sender.get_mut(trigger.entity) else {
        return;
    };

    message_sender.send::<VoxelChannel>(VoxelStateSync {
        modifications: modifications.modifications.clone(),
    });
}
