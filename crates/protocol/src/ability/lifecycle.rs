use super::types::{
    AbilityBulletOf, AbilityPhases, AbilityProjectileSpawn, ActiveAbility, ActiveBuffs, AoEHitbox,
    OnEndEffects, OnHitEffectDefs, OnHitEffects, OnInputEffects, OnTickEffects,
    ProjectileSpawnEffect, WhileActiveEffects,
};
use bevy::prelude::*;
use lightyear::prelude::LocalTimeline;

pub fn expire_buffs(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    mut query: Query<(Entity, &mut ActiveBuffs)>,
) {
    let tick = timeline.tick();
    for (entity, mut buffs) in &mut query {
        buffs.0.retain(|b| {
            let remaining: i16 = b.expires_tick - tick;
            remaining > 0
        });
        if buffs.0.is_empty() {
            commands.entity(entity).remove::<ActiveBuffs>();
        }
    }
}

pub fn cleanup_effect_markers_on_removal(
    trigger: On<Remove, ActiveAbility>,
    mut commands: Commands,
) {
    if let Ok(mut cmd) = commands.get_entity(trigger.entity) {
        cmd.try_remove::<OnTickEffects>();
        cmd.try_remove::<WhileActiveEffects>();
        cmd.try_remove::<OnHitEffects>();
        cmd.try_remove::<OnHitEffectDefs>();
        cmd.try_remove::<OnEndEffects>();
        cmd.try_remove::<OnInputEffects>();
        cmd.try_remove::<ProjectileSpawnEffect>();
        cmd.try_remove::<AbilityPhases>();
    }
}

pub fn aoe_hitbox_lifetime(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    query: Query<(Entity, &AoEHitbox)>,
) {
    let tick = timeline.tick();
    for (entity, aoe) in &query {
        let elapsed = tick - aoe.spawn_tick;
        if elapsed >= aoe.duration_ticks as i16 {
            commands.entity(entity).try_despawn();
        }
    }
}

pub fn ability_bullet_lifetime(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    query: Query<(Entity, &AbilityBulletOf)>,
    spawn_query: Query<&AbilityProjectileSpawn>,
) {
    let tick = timeline.tick();
    for (entity, bullet_of) in &query {
        if let Ok(spawn_info) = spawn_query.get(bullet_of.0) {
            let elapsed = tick - spawn_info.spawn_tick;
            if elapsed >= spawn_info.lifetime_ticks as i16 {
                commands.entity(entity).try_despawn();
            }
        }
    }
}
