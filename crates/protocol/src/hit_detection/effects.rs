use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::reflect::TypeRegistryArc;
use lightyear::prelude::{ControlledBy, Tick};

use crate::ability::{
    spawn_sub_ability, AbilityAsset, AbilityDefs, AbilityEffect, ActiveBuffs, ActiveShield,
    EffectTarget, ForceFrame, OnHitEffects,
};
use crate::{Health, Invulnerable, PlayerId};

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

pub(crate) fn apply_on_hit_effects(
    commands: &mut Commands,
    ability_defs: &AbilityDefs,
    ability_assets: &Assets<AbilityAsset>,
    registry: &TypeRegistryArc,
    tick: Tick,
    server_query: &Query<&ControlledBy>,
    player_id_query: &Query<&PlayerId>,
    on_hit: &OnHitEffects,
    victim: Entity,
    source_pos: Vec3,
    target_query: &mut Query<(
        &Position,
        Option<&mut LinearVelocity>,
        &mut Health,
        Option<&Invulnerable>,
    )>,
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
                if let Ok((target_pos, velocity, _, _)) = target_query.get_mut(entity) {
                    let world_force = resolve_force_frame(
                        *force,
                        frame,
                        source_pos,
                        target_pos.0,
                        on_hit.caster,
                        entity,
                        rotation_query,
                    );
                    if let Some(mut velocity) = velocity {
                        velocity.0 += world_force;
                    }
                } else {
                    warn!("ApplyForce target {:?} not found", entity);
                }
            }
            AbilityEffect::Ability { id, target } => {
                let target_entity = resolve_on_hit_target(target, victim, on_hit);
                spawn_sub_ability(
                    commands,
                    ability_defs,
                    ability_assets,
                    registry,
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
