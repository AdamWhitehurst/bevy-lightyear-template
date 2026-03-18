use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::types::{MapInstanceId, MapSwitchTarget};

/// Channel for map transition messages
pub struct MapChannel;

/// Client requests to switch maps
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct PlayerMapSwitchRequest {
    pub target: MapSwitchTarget,
}

/// Marks a player entity as undergoing a map transition.
/// Carried on the player entity on both client and server.
#[derive(Component, Clone, Debug)]
pub struct PendingTransition(pub MapInstanceId);

/// Server tells client to begin transition
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct MapTransitionStart {
    pub target: MapInstanceId,
    pub seed: u64,
    pub generation_version: u32,
    pub bounds: Option<IVec3>,
    pub spawn_position: Vec3,
}

/// Client tells server that chunks are loaded and it's ready
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct MapTransitionReady;

/// Server tells client the transition is complete, player is unfrozen
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct MapTransitionEnd;

/// Marker: client has sent MapTransitionReady, awaiting MapTransitionEnd
#[derive(Component)]
pub struct TransitionReadySent;
