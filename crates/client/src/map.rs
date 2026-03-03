use bevy::prelude::*;
use lightyear::prelude::{MessageReceiver, MessageSender};
use protocol::{VoxelChannel, VoxelEditBroadcast, VoxelEditRequest, VoxelStateSync, VoxelType};

/// Plugin managing client-side voxel map functionality
pub struct ClientMapPlugin;

impl Plugin for ClientMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (handle_voxel_broadcasts, handle_state_sync));
    }
}

fn handle_voxel_broadcasts(mut receiver: Query<&mut MessageReceiver<VoxelEditBroadcast>>) {
    for mut message_receiver in receiver.iter_mut() {
        for broadcast in message_receiver.receive() {
            warn!(
                "Voxel broadcast received but engine not yet integrated: {:?}",
                broadcast.position
            );
        }
    }
}

fn handle_state_sync(mut receiver: Query<&mut MessageReceiver<VoxelStateSync>>) {
    for mut message_receiver in receiver.iter_mut() {
        for sync in message_receiver.receive() {
            warn!(
                "Voxel state sync received ({} mods) but engine not yet integrated",
                sync.modifications.len()
            );
        }
    }
}

/// System for sending edit requests (called from gameplay systems)
pub fn send_voxel_edit(
    position: IVec3,
    voxel: VoxelType,
    mut message_sender: Query<&mut MessageSender<VoxelEditRequest>>,
) {
    for mut sender in message_sender.iter_mut() {
        sender.send::<VoxelChannel>(VoxelEditRequest { position, voxel });
    }
}
