use crate::hit_detection::character_collision_layers;
use crate::map::MapSaveTarget;
use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::{PeerId, Tick};
use serde::{Deserialize, Serialize};

pub const CHARACTER_CAPSULE_RADIUS: f32 = 2.0;
pub const CHARACTER_CAPSULE_HEIGHT: f32 = 2.0;

/// Identifies which client owns this character. Replicated to all clients so
/// shared systems (e.g. prespawn salt computation) can access the owner's
/// `PeerId` without server-only queries.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
#[type_path = "protocol"]
pub struct PlayerId(pub PeerId);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CharacterMarker;

/// Marker to distinguish dummy targets from player characters.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DummyTarget;

/// Determines which sprite rig and animation set a character uses on the client.
#[derive(
    Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, Default,
)]
#[type_path = "protocol"]
pub enum CharacterType {
    #[default]
    Humanoid,
}

/// Marks a respawn location. Server-only, not replicated.
#[derive(Component, Clone, Debug)]
#[require(MapSaveTarget)]
pub struct RespawnPoint;

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Default)]
#[type_path = "protocol"]
#[reflect(Component, Default)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    pub fn new(max: f32) -> Self {
        Self { current: max, max }
    }

    pub fn apply_damage(&mut self, damage: f32) {
        self.current = (self.current - damage).max(0.0);
    }

    pub fn is_dead(&self) -> bool {
        self.current <= 0.0
    }

    pub fn restore_full(&mut self) {
        self.current = self.max;
    }
}

/// Post-respawn invulnerability. Prevents damage while present.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Invulnerable {
    pub expires_at: Tick,
}

/// Default respawn delay when no `RespawnTimerConfig` is present.
pub const DEFAULT_RESPAWN_TICKS: u16 = 256;

/// Per-entity configuration for respawn delay. Loadable from RON.
#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[type_path = "protocol"]
#[reflect(Component, Serialize, Deserialize)]
pub struct RespawnTimerConfig {
    pub duration_ticks: u16,
}

impl Default for RespawnTimerConfig {
    fn default() -> Self {
        Self {
            duration_ticks: DEFAULT_RESPAWN_TICKS,
        }
    }
}

/// Marks an entity as dead and awaiting respawn. Inserted when Health reaches 0.
/// Removed when the timer expires and respawn occurs.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RespawnTimer {
    pub expires_at: Tick,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ColorComponent(pub Color);

#[derive(Bundle)]
pub struct CharacterPhysicsBundle {
    pub collider: Collider,
    pub rigid_body: RigidBody,
    pub locked_axes: LockedAxes,
    pub friction: Friction,
    pub collision_layers: CollisionLayers,
}

impl Default for CharacterPhysicsBundle {
    fn default() -> Self {
        Self {
            collider: Collider::capsule(CHARACTER_CAPSULE_RADIUS, CHARACTER_CAPSULE_HEIGHT),
            rigid_body: RigidBody::Dynamic,
            locked_axes: LockedAxes::ROTATION_LOCKED,
            friction: Friction::default(),
            collision_layers: character_collision_layers(),
        }
    }
}
