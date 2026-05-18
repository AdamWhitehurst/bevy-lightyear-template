use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::NetworkedPlayerActions;

/// Semantic ability-domain input used by ability assets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[type_path = "protocol::ability"]
pub enum AbilityInput {
    Slot(usize),
    Jump,
}

impl AbilityInput {
    /// Maps semantic ability input onto the filtered network transport action.
    pub fn to_networked_action(self) -> Option<NetworkedPlayerActions> {
        match self {
            Self::Slot(0) => Some(NetworkedPlayerActions::Ability1),
            Self::Slot(1) => Some(NetworkedPlayerActions::Ability2),
            Self::Slot(2) => Some(NetworkedPlayerActions::Ability3),
            Self::Slot(3) => Some(NetworkedPlayerActions::Ability4),
            Self::Slot(_) => None,
            Self::Jump => Some(NetworkedPlayerActions::Jump),
        }
    }
}
