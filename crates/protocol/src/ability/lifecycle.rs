use super::types::{
    AbilityBulletOf, AbilityProjectileSpawn, ActiveAbility, ActiveBuffs, AoEHitbox,
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

/// Despawn the entity whenever its `ActiveAbility` component is removed.
///
/// Two important paths trigger this:
/// 1. **Lightyear rollback strip.** `prepare_rollback` iterates predicted/PreSpawned entities
///    and removes any `SyncComponent` whose `PredictionHistory` has no entry at the rollback
///    target tick. For an `ActiveAbility` spawned at T_press and a rollback to T_press-1,
///    that strip leaves the entity alive but without its driving component — and replay's
///    activation would then `commands.spawn(...)` a new one, leaving us with a duplicate.
///    Despawning here makes the rollback semantically equivalent to "this cast didn't
///    happen," so replay's spawn is the only one that exists afterward.
/// 2. **Server rejecting a predicted cast.** Server's authoritative state at the rollback
///    target tick may contain no `ActiveAbility` for this entity (player wasn't grounded,
///    cooldown was actually still active server-side, etc.). The component is removed during
///    rollback; the entity is despawned here. The predicted cast is fully rolled back.
///
/// On natural end-of-life (`prediction_despawn` at end of `Recovery`), `ActiveAbility` is
/// NOT removed — the entity is tagged `PredictionDisable` and lingers until the server's
/// confirmed despawn arrives. When that confirmed despawn finally removes the entity, all
/// components (including `ActiveAbility`) come off in one shot; this observer's `try_despawn`
/// then runs against an entity that's already being despawned and is a harmless no-op.
///
/// `HitboxOf`/`AbilityBulletOf` are `linked_spawn` relationships rooted at this entity,
/// so this despawn cascades to clean up melee/AoE hitboxes and projectile bullets the cast
/// produced. Those entities carry `DisableRollback` (their per-tick state isn't rolled back
/// individually), but cascade-despawn via the parent is the right semantic for a
/// rolled-back cast: hitboxes shouldn't keep applying damage from a cast the server
/// overruled. If the cast is re-predicted at the same tick during replay, hitboxes are
/// re-spawned by `apply_on_tick_effects` at the appropriate `active_offset`.
pub fn despawn_active_ability_on_removal(
    trigger: On<Remove, ActiveAbility>,
    mut commands: Commands,
) {
    if let Ok(mut cmd) = commands.get_entity(trigger.entity) {
        cmd.try_despawn();
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
