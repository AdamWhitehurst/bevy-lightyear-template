use avian3d::prelude::*;
use bevy::prelude::*;

use lightyear::prelude::server::ClientOf;
use lightyear::prelude::{ControlledBy, LocalTimeline, NetworkTimeline, Tick};

use crate::ability::{
    facing_direction, spawn_sub_ability, AbilityBulletOf, AbilityDefs, AbilityEffect, AbilityPhase,
    ActiveAbility, ActiveBuffs, ActiveShield, EffectTarget, ForceFrame, HitTargets, HitboxOf,
    MeleeHitbox, OnHitEffects,
};
use crate::{CharacterMarker, Health, Invulnerable, PlayerId};

pub const MELEE_HITBOX_OFFSET: f32 = 1.5;
pub const MELEE_HITBOX_HALF_EXTENTS: Vec3 = Vec3::new(0.75, 1.0, 0.5);

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

/// Collision layer config for hitbox entities (melee/AoE).
pub fn hitbox_collision_layers() -> CollisionLayers {
    CollisionLayers::new(GameLayer::Hitbox, [GameLayer::Character])
}

/// Update melee hitbox positions to follow caster's position + facing offset.
pub fn update_hitbox_positions(
    mut hitbox_query: Query<(&HitboxOf, &mut Position, &mut Rotation), With<MeleeHitbox>>,
    ability_query: Query<&ActiveAbility>,
    caster_query: Query<(&Position, &Rotation), Without<MeleeHitbox>>,
) {
    for (hitbox_of, mut hitbox_pos, mut hitbox_rot) in &mut hitbox_query {
        let Ok(active) = ability_query.get(hitbox_of.0) else {
            continue;
        };
        let Ok((caster_pos, caster_rot)) = caster_query.get(active.caster) else {
            continue;
        };
        let direction = facing_direction(caster_rot);
        hitbox_pos.0 = caster_pos.0 + direction * MELEE_HITBOX_OFFSET;
        *hitbox_rot = *caster_rot;
    }
}

/// Detect hits from hitbox entities (melee and AoE) using `CollidingEntities`.
pub fn process_hitbox_hits(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    server_query: Query<&ControlledBy>,
    player_id_query: Query<&PlayerId>,
    mut hitbox_query: Query<(
        &CollidingEntities,
        &OnHitEffects,
        &mut HitTargets,
        &Position,
    )>,
    mut target_query: Query<
        (
            &Position,
            &mut LinearVelocity,
            &mut Health,
            Option<&Invulnerable>,
        ),
        With<CharacterMarker>,
    >,
    mut shield_query: Query<&mut ActiveShield>,
    buff_query: Query<&ActiveBuffs>,
    rotation_query: Query<&Rotation>,
) {
    let tick = timeline.tick();
    for (colliding, on_hit, mut hit_targets, hitbox_pos) in &mut hitbox_query {
        for &target in colliding.iter() {
            if target == on_hit.caster || target == on_hit.original_caster {
                continue;
            }
            if !hit_targets.0.insert(target) {
                continue;
            }
            if target_query.get(target).is_err() {
                continue;
            }
            apply_on_hit_effects(
                &mut commands,
                ability_defs.as_ref(),
                tick,
                &server_query,
                &player_id_query,
                on_hit,
                target,
                hitbox_pos.0,
                &mut target_query,
                &mut shield_query,
                &buff_query,
                &rotation_query,
            );
        }
    }
}

/// Despawn hitbox entities when their parent ability leaves Active phase.
pub fn cleanup_hitbox_entities(
    mut commands: Commands,
    hitbox_query: Query<(Entity, &HitboxOf)>,
    ability_query: Query<&ActiveAbility>,
) {
    for (hitbox_entity, hitbox_of) in &hitbox_query {
        let should_despawn = match ability_query.get(hitbox_of.0) {
            Ok(active) => active.phase != AbilityPhase::Active,
            Err(_) => true,
        };
        if should_despawn {
            commands.entity(hitbox_entity).try_despawn();
        }
    }
}

/// Detect projectile hits via CollidingEntities and apply on-hit effects.
pub fn process_projectile_hits(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    server_query: Query<&ControlledBy>,
    player_id_query: Query<&PlayerId>,
    bullet_query: Query<
        (Entity, &CollidingEntities, &OnHitEffects, &Position),
        With<AbilityBulletOf>,
    >,
    mut target_query: Query<
        (
            &Position,
            &mut LinearVelocity,
            &mut Health,
            Option<&Invulnerable>,
        ),
        With<CharacterMarker>,
    >,
    mut shield_query: Query<&mut ActiveShield>,
    buff_query: Query<&ActiveBuffs>,
    rotation_query: Query<&Rotation>,
) {
    let tick = timeline.tick();
    for (bullet, colliding, on_hit, bullet_pos) in &bullet_query {
        for &target in colliding.iter() {
            if target == on_hit.original_caster {
                continue;
            }
            if target_query.get(target).is_err() {
                continue;
            }
            apply_on_hit_effects(
                &mut commands,
                ability_defs.as_ref(),
                tick,
                &server_query,
                &player_id_query,
                on_hit,
                target,
                bullet_pos.0,
                &mut target_query,
                &mut shield_query,
                &buff_query,
                &rotation_query,
            );
            commands.entity(bullet).try_despawn();
            break;
        }
    }
}

fn resolve_on_hit_target(target: &EffectTarget, victim: Entity, on_hit: &OnHitEffects) -> Entity {
    match target {
        EffectTarget::Victim => victim,
        EffectTarget::Caster => on_hit.caster,
        EffectTarget::OriginalCaster => on_hit.original_caster,
    }
}

/// Apply "damage" stat buffs from the caster to a base damage amount.
fn apply_damage_buffs(base: f32, caster: Entity, buff_query: &Query<&ActiveBuffs>) -> f32 {
    let Ok(buffs) = buff_query.get(caster) else {
        return base;
    };
    let multiplier: f32 = buffs
        .0
        .iter()
        .filter(|b| b.stat == "damage")
        .map(|b| b.multiplier)
        .product();
    base * multiplier
}

fn resolve_force_frame(
    force: Vec3,
    frame: &ForceFrame,
    caster_pos: Vec3,
    victim_pos: Vec3,
    caster: Entity,
    victim: Entity,
    rotation_query: &Query<&Rotation>,
) -> Vec3 {
    match frame {
        ForceFrame::World => force,
        ForceFrame::Caster => rotation_query.get(caster).map(|r| r.0).unwrap_or_default() * force,
        ForceFrame::Victim => rotation_query.get(victim).map(|r| r.0).unwrap_or_default() * force,
        ForceFrame::RelativePosition => {
            let forward = (victim_pos - caster_pos).normalize_or(Vec3::Z);
            let right = Vec3::Y.cross(forward).normalize_or(Vec3::X);
            let up = forward.cross(right);
            Quat::from_mat3(&Mat3::from_cols(right, up, forward)) * force
        }
        ForceFrame::RelativeRotation => {
            let cr = rotation_query.get(caster).map(|r| r.0).unwrap_or_default();
            let vr = rotation_query.get(victim).map(|r| r.0).unwrap_or_default();
            (vr * cr.inverse()) * force
        }
    }
}

fn apply_on_hit_effects(
    commands: &mut Commands,
    ability_defs: &AbilityDefs,
    tick: Tick,
    server_query: &Query<&ControlledBy>,
    player_id_query: &Query<&PlayerId>,
    on_hit: &OnHitEffects,
    victim: Entity,
    source_pos: Vec3,
    target_query: &mut Query<
        (
            &Position,
            &mut LinearVelocity,
            &mut Health,
            Option<&Invulnerable>,
        ),
        With<CharacterMarker>,
    >,
    shield_query: &mut Query<&mut ActiveShield>,
    buff_query: &Query<&ActiveBuffs>,
    rotation_query: &Query<&Rotation>,
) {
    for effect in &on_hit.effects {
        match effect {
            AbilityEffect::Damage { amount, target } => {
                let entity = resolve_on_hit_target(target, victim, on_hit);
                let mut remaining_damage = apply_damage_buffs(*amount, on_hit.caster, buff_query);

                if let Ok(mut shield) = shield_query.get_mut(entity) {
                    if shield.remaining >= remaining_damage {
                        shield.remaining -= remaining_damage;
                        continue;
                    }
                    remaining_damage -= shield.remaining;
                    shield.remaining = 0.0;
                    commands.entity(entity).remove::<ActiveShield>();
                }

                if let Ok((_, _, mut health, invulnerable)) = target_query.get_mut(entity) {
                    if invulnerable.is_none() {
                        health.apply_damage(remaining_damage);
                    }
                } else {
                    warn!("Damage target {:?} not found", entity);
                }
            }
            AbilityEffect::ApplyForce {
                force,
                frame,
                target,
            } => {
                let entity = resolve_on_hit_target(target, victim, on_hit);
                if let Ok((target_pos, mut velocity, _, _)) = target_query.get_mut(entity) {
                    let world_force = resolve_force_frame(
                        *force,
                        frame,
                        source_pos,
                        target_pos.0,
                        on_hit.caster,
                        entity,
                        rotation_query,
                    );
                    velocity.0 += world_force;
                } else {
                    warn!("ApplyForce target {:?} not found", entity);
                }
            }
            AbilityEffect::Ability { id, target } => {
                let target_entity = resolve_on_hit_target(target, victim, on_hit);
                spawn_sub_ability(
                    commands,
                    ability_defs,
                    id,
                    target_entity,
                    on_hit.original_caster,
                    0,
                    on_hit.depth,
                    tick,
                    server_query,
                    player_id_query,
                );
            }
            _ => {
                warn!("Unhandled OnHit effect: {:?}", effect);
            }
        }
    }
}
