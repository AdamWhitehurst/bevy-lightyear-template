mod effects;
mod layers;
mod systems;

pub use layers::{
    character_collision_layers, damageable_collision_layers, hitbox_collision_layers,
    projectile_collision_layers, terrain_collision_layers, GameLayer, MELEE_HITBOX_HALF_EXTENTS,
    MELEE_HITBOX_OFFSET,
};
pub use systems::{
    cleanup_hitbox_entities, process_hitbox_hits, process_projectile_hits, update_hitbox_positions,
};
