use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::map::{MapInstanceId, MapTransitionStart};
use voxel_map_engine::lifecycle::world_to_column_pos;

/// Client-side transition phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Reflect)]
pub enum TransitionPhase {
    #[default]
    Idle,
    Cleanup,
    Loading,
    Ready,
    Complete,
}

/// Client-side transition state resource.
#[derive(Resource, Debug)]
pub struct ClientTransitionState {
    pub phase: TransitionPhase,
    pub target_map: Option<MapInstanceId>,
    pub readiness_radius: u32,
    pub spawn_position: Vec3,
    /// Chunk-space column derived from spawn_position via world_to_column_pos.
    pub spawn_column: IVec2,
    pub chunk_size: u32,
    pub column_y_range: (i32, i32),
    /// Raw server Entity IDs from MapTransitionEntity messages.
    /// Lightyear does NOT auto-remap these — remapping only happens if
    /// the message type implements MapEntities AND .add_map_entities()
    /// is chained on registration. We skip both deliberately.
    pub pending_entities: Vec<Entity>,
    pub end_received: bool,
}

impl Default for ClientTransitionState {
    fn default() -> Self {
        Self {
            phase: TransitionPhase::Idle,
            target_map: None,
            readiness_radius: 0,
            spawn_position: Vec3::ZERO,
            spawn_column: IVec2::ZERO,
            chunk_size: 1,
            column_y_range: (0, 0),
            pending_entities: Vec::new(),
            end_received: false,
        }
    }
}

impl ClientTransitionState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Initialize from a MapTransitionStart message.
    pub fn begin(&mut self, start: &MapTransitionStart) {
        self.phase = TransitionPhase::Cleanup;
        self.target_map = Some(start.target.clone());
        self.readiness_radius = start.readiness_radius;
        self.spawn_position = start.spawn_position;
        self.chunk_size = start.chunk_size;
        self.column_y_range = start.column_y_range;
        self.pending_entities.clear();
        self.end_received = false;
        self.spawn_column = world_to_column_pos(start.spawn_position, start.chunk_size);
    }
}

/// Server→Client message carrying an unmapped server-side Entity ID for a
/// relocated entity. We deliberately skip MapEntities + add_map_entities()
/// so the server Entity arrives unchanged on the client. Client polls
/// RemoteEntityMap::get_local until the mapping resolves.
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
#[type_path = "protocol::transition"]
pub struct MapTransitionEntity {
    pub entity: Entity,
}

/// Inserted on the player entity after server Phase 1 completes.
/// Carries data needed for Phase 2 (complete_map_transition).
#[derive(Component)]
pub struct TransitionPending {
    pub client_entity: Entity,
    pub target_map_id: MapInstanceId,
    pub new_room: Entity,
    /// Entities removed from old room in Phase 1 that need AddEntity in Phase 2.
    pub relocated_entities: Vec<Entity>,
}

/// Default readiness radius (Chebyshev column distance from spawn).
pub const TRANSITION_READINESS_RADIUS: u32 = 2;
