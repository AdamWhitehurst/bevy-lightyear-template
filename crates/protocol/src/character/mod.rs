pub mod movement;
pub mod types;

pub use movement::{apply_movement, update_facing};
pub use types::{
    CharacterMarker, CharacterPhysicsBundle, CharacterType, ColorComponent, DeathEvent,
    DummyTarget, Health, Invulnerable, PlayerId, RespawnPoint, RespawnTimer, RespawnTimerConfig,
    CHARACTER_CAPSULE_HEIGHT, CHARACTER_CAPSULE_RADIUS, DEFAULT_RESPAWN_TICKS,
};
