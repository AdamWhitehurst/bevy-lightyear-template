use avian3d::prelude::*;
use bevy::prelude::Vec3;

pub const MELEE_HITBOX_OFFSET: f32 = 3.0;
pub const MELEE_HITBOX_HALF_EXTENTS: Vec3 = Vec3::new(1.5, 2.0, 1.0);

#[derive(PhysicsLayer, Default)]
pub enum GameLayer {
    #[default]
    Default,
    Character,
    Hitbox,
    Projectile,
    Terrain,
    Damageable,
}

/// Collision layer config for characters.
pub fn character_collision_layers() -> CollisionLayers {
    CollisionLayers::new(
        GameLayer::Character,
        [
            GameLayer::Character,
            GameLayer::Terrain,
            GameLayer::Hitbox,
            GameLayer::Projectile,
            GameLayer::Damageable,
        ],
    )
}

/// Collision layer config for terrain.
pub fn terrain_collision_layers() -> CollisionLayers {
    CollisionLayers::new(GameLayer::Terrain, [GameLayer::Character])
}

/// Collision layer config for projectiles.
pub fn projectile_collision_layers() -> CollisionLayers {
    CollisionLayers::new(
        GameLayer::Projectile,
        [GameLayer::Character, GameLayer::Damageable],
    )
}

/// Collision layer config for hitbox entities (melee/AoE).
pub fn hitbox_collision_layers() -> CollisionLayers {
    CollisionLayers::new(
        GameLayer::Hitbox,
        [GameLayer::Character, GameLayer::Damageable],
    )
}

/// Collision layer config for damageable world objects.
pub fn damageable_collision_layers() -> CollisionLayers {
    CollisionLayers::new(
        GameLayer::Damageable,
        [
            GameLayer::Character,
            GameLayer::Hitbox,
            GameLayer::Projectile,
        ],
    )
}
