use bevy::prelude::*;
use protocol::{VoxelBrushEditRequest, VoxelEditRequest};

/// Client-local terrain command intents produced after ownership gating.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum TerrainCommandIntent {
    BrushStroke(VoxelBrushEditRequest),
    LegacyVoxelEdit(VoxelEditRequest),
}
