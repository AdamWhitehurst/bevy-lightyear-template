use avian3d::prelude::*;
use bevy::ecs::message::MessageWriter;
use bevy::prelude::*;
use lightyear::prelude::{ControlledBy, LocalTimeline};

use super::effects::apply_on_hit_effects;
use super::layers::MELEE_HITBOX_OFFSET;
use crate::ability::{
    facing_direction, AbilityAsset, AbilityBulletOf, AbilityDefs, AbilityPhase, ActiveAbility,
    ActiveBuffs, ActiveShield, AoEHitbox, HitTargets, HitboxOf, MeleeHitbox, OnHitEffects,
};
use crate::{DeathEvent, Health, Invulnerable, PlayerId};

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
    ability_assets: Res<Assets<AbilityAsset>>,
    registry: Res<AppTypeRegistry>,
    timeline: Res<LocalTimeline>,
    server_query: Query<&ControlledBy>,
    player_id_query: Query<&PlayerId>,
    mut hitbox_query: Query<(
        &CollidingEntities,
        &OnHitEffects,
        &mut HitTargets,
        &Position,
    )>,
    mut target_query: Query<(
        &Position,
        Option<&mut LinearVelocity>,
        &mut Health,
        Option<&Invulnerable>,
    )>,
    mut shield_query: Query<&mut ActiveShield>,
    buff_query: Query<&ActiveBuffs>,
    rotation_query: Query<&Rotation>,
    mut death_events: MessageWriter<DeathEvent>,
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
                ability_assets.as_ref(),
                &registry.0,
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
                &mut death_events,
            );
        }
    }
}

/// Despawn hitbox entities when their parent ability leaves Active phase.
/// AoE hitboxes are excluded — their lifetime is governed by `duration_ticks`
/// via `aoe_hitbox_lifetime`, which can outlive the parent's Active phase.
pub fn cleanup_hitbox_entities(
    mut commands: Commands,
    hitbox_query: Query<(Entity, &HitboxOf), Without<AoEHitbox>>,
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
    ability_assets: Res<Assets<AbilityAsset>>,
    registry: Res<AppTypeRegistry>,
    timeline: Res<LocalTimeline>,
    server_query: Query<&ControlledBy>,
    player_id_query: Query<&PlayerId>,
    bullet_query: Query<
        (Entity, &CollidingEntities, &OnHitEffects, &Position),
        With<AbilityBulletOf>,
    >,
    mut target_query: Query<(
        &Position,
        Option<&mut LinearVelocity>,
        &mut Health,
        Option<&Invulnerable>,
    )>,
    mut shield_query: Query<&mut ActiveShield>,
    buff_query: Query<&ActiveBuffs>,
    rotation_query: Query<&Rotation>,
    mut death_events: MessageWriter<DeathEvent>,
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
                ability_assets.as_ref(),
                &registry.0,
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
                &mut death_events,
            );
            commands.entity(bullet).try_despawn();
            break;
        }
    }
}
