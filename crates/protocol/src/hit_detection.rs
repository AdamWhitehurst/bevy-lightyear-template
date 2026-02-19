use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::utils::collections::EntityHashSet;

use crate::ability::{facing_direction, MeleeHitTargets, MeleeHitboxActive};
use crate::{CharacterMarker, Health, Invulnerable};

/// Knockback force stored on a projectile entity.
#[derive(Component, Clone, Debug)]
pub struct KnockbackForce(pub f32);

/// Who shot this projectile (to prevent self-hits).
#[derive(Component, Clone, Debug)]
pub struct ProjectileOwner(pub Entity);

/// Damage stored on a projectile entity.
#[derive(Component, Clone, Debug)]
pub struct DamageAmount(pub f32);

const MELEE_HITBOX_OFFSET: f32 = 1.5;
const MELEE_HITBOX_HALF_EXTENTS: Vec3 = Vec3::new(0.75, 1.0, 0.5);

#[derive(PhysicsLayer, Default)]
pub enum GameLayer {
    #[default]
    Default,
    Character,
    Hitbox,
    Projectile,
    Terrain,
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
        ],
    )
}

/// Collision layer config for terrain.
pub fn terrain_collision_layers() -> CollisionLayers {
    CollisionLayers::new(GameLayer::Terrain, [GameLayer::Character])
}

/// Collision layer config for projectiles.
pub fn projectile_collision_layers() -> CollisionLayers {
    CollisionLayers::new(GameLayer::Projectile, [GameLayer::Character])
}

/// Insert MeleeHitTargets for characters that have MeleeHitboxActive but no targets yet.
pub fn ensure_melee_hit_targets(
    mut commands: Commands,
    query: Query<Entity, (With<MeleeHitboxActive>, Without<MeleeHitTargets>)>,
) {
    for entity in &query {
        commands.entity(entity).insert(MeleeHitTargets::default());
    }
}

/// Detect melee hits using one-shot spatial query each tick.
pub fn process_melee_hits(
    spatial_query: SpatialQuery,
    mut attacker_query: Query<
        (
            Entity,
            &MeleeHitboxActive,
            &mut MeleeHitTargets,
            &Position,
            &Rotation,
        ),
        With<CharacterMarker>,
    >,
    mut target_query: Query<(&Position, &mut LinearVelocity, &mut Health, Option<&Invulnerable>), With<CharacterMarker>>,
) {
    for (entity, hitbox, mut hit_targets, pos, rot) in &mut attacker_query {
        let direction = facing_direction(rot);
        let hitbox_pos = pos.0 + direction * MELEE_HITBOX_OFFSET;

        let filter = SpatialQueryFilter {
            mask: GameLayer::Character.into(),
            excluded_entities: EntityHashSet::from_iter([entity]),
        };

        let hits = spatial_query.shape_intersections(
            &Collider::cuboid(
                MELEE_HITBOX_HALF_EXTENTS.x,
                MELEE_HITBOX_HALF_EXTENTS.y,
                MELEE_HITBOX_HALF_EXTENTS.z,
            ),
            hitbox_pos,
            rot.0,
            &filter,
        );

        for target in hits {
            if !hit_targets.0.insert(target) {
                continue; // already hit
            }
            apply_hit(&mut target_query, target, pos.0, hitbox.knockback_force, hitbox.base_damage);
        }
    }
}

/// Detect projectile hits via CollidingEntities and apply knockback.
pub fn process_projectile_hits(
    mut commands: Commands,
    bullet_query: Query<
        (Entity, &CollidingEntities, &KnockbackForce, &DamageAmount, &ProjectileOwner, &Position),
        With<Sensor>,
    >,
    mut target_query: Query<(&Position, &mut LinearVelocity, &mut Health, Option<&Invulnerable>), With<CharacterMarker>>,
) {
    for (bullet, colliding, knockback, damage, owner, bullet_pos) in &bullet_query {
        for &target in colliding.iter() {
            if target == owner.0 {
                continue;
            }
            if target_query.get(target).is_err() {
                continue;
            }
            apply_hit(&mut target_query, target, bullet_pos.0, knockback.0, damage.0);
            commands.entity(bullet).try_despawn();
            break; // bullet hits one target
        }
    }
}

fn apply_hit(
    target_query: &mut Query<(&Position, &mut LinearVelocity, &mut Health, Option<&Invulnerable>), With<CharacterMarker>>,
    target: Entity,
    source_pos: Vec3,
    knockback_force: f32,
    damage: f32,
) {
    let Ok((target_pos, mut velocity, mut health, invulnerable)) = target_query.get_mut(target) else {
        return;
    };
    let horizontal = (target_pos.0 - source_pos).with_y(0.0);
    let direction = if horizontal.length() > 0.01 {
        (horizontal.normalize() + Vec3::Y * 0.3).normalize()
    } else {
        Vec3::Y
    };
    velocity.0 += direction * knockback_force;
    if invulnerable.is_none() {
        health.apply_damage(damage);
    }
}
