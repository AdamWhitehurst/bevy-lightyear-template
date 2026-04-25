use super::loader::apply_ability_archetype;
use super::types::facing_direction;
use super::types::{
    AbilityAsset, AbilityBulletOf, AbilityBullets, AbilityDefs, AbilityId, AbilityPhase,
    AbilityProjectileSpawn, ActiveAbility, AoEHitbox, HitTargets, HitboxOf, MeleeHitbox,
    OnHitEffects, ProjectileSpawnEffect,
};
use crate::hit_detection::{
    hitbox_collision_layers, projectile_collision_layers, MELEE_HITBOX_HALF_EXTENTS,
    MELEE_HITBOX_OFFSET,
};
use crate::map::MapInstanceId;
use crate::PlayerId;
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::reflect::TypeRegistryArc;
use lightyear::prelude::{
    ControlledBy, DisableRollback, LocalTimeline, NetworkTarget, PreSpawned,
    PredictionDespawnCommandsExt, PredictionTarget, Replicate, Replicated, Tick,
};
use std::hash::{DefaultHasher, Hash, Hasher};

const PROJECTILE_SPAWN_OFFSET: f32 = 3.0;
const BULLET_COLLIDER_RADIUS: f32 = 0.5;

fn compute_sub_ability_salt(player_id: PlayerId, slot: u8, depth: u8, id: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    player_id.0.to_bits().hash(&mut hasher);
    slot.hash(&mut hasher);
    depth.hash(&mut hasher);
    id.hash(&mut hasher);
    hasher.finish()
}

/// Spawn a sub-ability entity for recursive ability composition.
pub(crate) fn spawn_sub_ability(
    commands: &mut Commands,
    ability_defs: &AbilityDefs,
    ability_assets: &Assets<AbilityAsset>,
    registry: &TypeRegistryArc,
    id: &str,
    target_entity: Entity,
    original_caster: Entity,
    parent_slot: u8,
    parent_depth: u8,
    tick: Tick,
    server_query: &Query<&ControlledBy>,
    player_id_query: &Query<&PlayerId>,
) {
    if parent_depth >= 4 {
        warn!("Ability recursion depth exceeded for {:?}", id);
        return;
    }
    let ability_id = AbilityId(id.to_string());
    let Some(handle) = ability_defs.get(&ability_id) else {
        warn!("Sub-ability {:?} not found in defs", id);
        return;
    };
    let Some(asset) = ability_assets.get(handle) else {
        warn!("Sub-ability {:?} asset not loaded", id);
        return;
    };
    let Some(&player_id) = player_id_query.get(original_caster).ok() else {
        warn!(
            "Sub-ability spawn: original_caster {:?} missing PlayerId",
            original_caster
        );
        return;
    };
    let depth = parent_depth + 1;
    let salt = compute_sub_ability_salt(player_id, parent_slot, depth, id);

    let entity_id = commands
        .spawn((
            ActiveAbility {
                def_id: ability_id,
                caster: target_entity,
                original_caster,
                target: target_entity,
                phase: AbilityPhase::Startup,
                phase_start_tick: tick,
                ability_slot: parent_slot,
                depth,
            },
            PreSpawned::default_with_salt(salt),
            Name::new("ActiveAbility"),
        ))
        .id();

    apply_ability_archetype(commands, entity_id, asset, registry.clone(), Vec::new());

    if let Ok(controlled_by) = server_query.get(original_caster) {
        commands.entity(entity_id).insert((
            Replicate::to_clients(NetworkTarget::All),
            PredictionTarget::to_clients(NetworkTarget::All),
            *controlled_by,
        ));
    }
}

pub(crate) fn spawn_melee_hitbox(
    commands: &mut Commands,
    ability_entity: Entity,
    active: &ActiveAbility,
    on_hit_effects: Option<&OnHitEffects>,
    caster_query: &Query<(&mut Position, &Rotation, &MapInstanceId)>,
) {
    let Ok((caster_pos, caster_rot, caster_map_id)) = caster_query.get(active.caster) else {
        warn!(
            "Melee hitbox spawn: caster {:?} missing Position/Rotation",
            active.caster
        );
        return;
    };
    let direction = facing_direction(caster_rot);
    let pos = caster_pos.0 + direction * MELEE_HITBOX_OFFSET;

    let mut cmd = commands.spawn((
        Position(pos),
        *caster_rot,
        RigidBody::Kinematic,
        Collider::cuboid(
            MELEE_HITBOX_HALF_EXTENTS.x,
            MELEE_HITBOX_HALF_EXTENTS.y,
            MELEE_HITBOX_HALF_EXTENTS.z,
        ),
        Sensor,
        CollisionEventsEnabled,
        CollidingEntities::default(),
        hitbox_collision_layers(),
        HitboxOf(ability_entity),
        DisableRollback,
        MeleeHitbox,
        HitTargets::default(),
        Name::new("MeleeHitbox"),
    ));
    if let Some(on_hit) = on_hit_effects {
        cmd.insert(on_hit.clone());
    }
    cmd.insert(caster_map_id.clone());
}

pub(crate) fn spawn_aoe_hitbox(
    commands: &mut Commands,
    ability_entity: Entity,
    active: &ActiveAbility,
    on_hit_effects: Option<&OnHitEffects>,
    caster_query: &Query<(&mut Position, &Rotation, &MapInstanceId)>,
    radius: f32,
    spawn_tick: Tick,
    duration_ticks: u16,
) {
    trace!("Spawning AoE hitbox with {duration_ticks:?} lifetime");
    let Ok((caster_pos, caster_rot, caster_map_id)) = caster_query.get(active.caster) else {
        warn!(
            "AoE hitbox spawn: caster {:?} missing Position/Rotation",
            active.caster
        );
        return;
    };

    let mut cmd = commands.spawn((
        Position(caster_pos.0),
        *caster_rot,
        RigidBody::Kinematic,
        Collider::sphere(radius),
        Sensor,
        CollisionEventsEnabled,
        CollidingEntities::default(),
        hitbox_collision_layers(),
        HitboxOf(ability_entity),
        DisableRollback,
        HitTargets::default(),
        AoEHitbox {
            spawn_tick,
            duration_ticks,
        },
        Name::new("AoEHitbox"),
    ));
    if let Some(on_hit) = on_hit_effects {
        cmd.insert(on_hit.clone());
    }
    cmd.insert(caster_map_id.clone());
}

pub fn ability_projectile_spawn(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    query: Query<(
        Entity,
        &ProjectileSpawnEffect,
        &ActiveAbility,
        Option<&OnHitEffects>,
    )>,
    caster_query: Query<(&Position, &Rotation, &MapInstanceId)>,
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();

    for (ability_entity, request, active, on_hit_effects) in &query {
        let Ok((position, rotation, caster_map_id)) = caster_query.get(active.caster) else {
            warn!(
                "Projectile spawn: caster {:?} missing Position/Rotation",
                active.caster
            );
            continue;
        };
        let direction = facing_direction(rotation);
        let spawn_info = AbilityProjectileSpawn {
            spawn_tick: tick,
            position: position.0 + direction * PROJECTILE_SPAWN_OFFSET,
            direction,
            speed: request.speed,
            lifetime_ticks: request.lifetime_ticks,
            ability_id: active.def_id.clone(),
            shooter: active.caster,
        };

        let salt = (active.ability_slot as u64) << 8 | (active.depth as u64);
        let mut cmd = commands.spawn((
            spawn_info,
            PreSpawned::default_with_salt(salt),
            Name::new("AbilityProjectileSpawn"),
        ));

        if let Some(on_hit) = on_hit_effects {
            cmd.insert(on_hit.clone());
        }
        cmd.insert(caster_map_id.clone());

        if let Ok(controlled_by) = server_query.get(active.caster) {
            cmd.insert((
                Replicate::to_clients(NetworkTarget::All),
                PredictionTarget::to_clients(NetworkTarget::All),
                *controlled_by,
            ));
        }

        commands
            .entity(ability_entity)
            .remove::<ProjectileSpawnEffect>();
    }
}

pub fn handle_ability_projectile_spawn(
    mut commands: Commands,
    spawn_query: Query<
        (
            Entity,
            &AbilityProjectileSpawn,
            Option<&OnHitEffects>,
            &MapInstanceId,
        ),
        (Without<AbilityBullets>, Without<Replicated>),
    >,
) {
    for (spawn_entity, spawn_info, on_hit_effects, spawn_map_id) in &spawn_query {
        trace!("Spawning ability bullet from {:?}", spawn_info.ability_id);
        let mut bullet_cmd = commands.spawn((
            Position(spawn_info.position),
            Rotation::default(),
            LinearVelocity(spawn_info.direction * spawn_info.speed),
            RigidBody::Kinematic,
            Collider::sphere(BULLET_COLLIDER_RADIUS),
            Sensor,
            CollisionEventsEnabled,
            CollidingEntities::default(),
            projectile_collision_layers(),
            AbilityBulletOf(spawn_entity),
            DisableRollback,
            Name::new("AbilityBullet"),
        ));
        if let Some(on_hit) = on_hit_effects {
            bullet_cmd.insert(on_hit.clone());
        }
        bullet_cmd.insert(spawn_map_id.clone());
    }
}

pub fn despawn_ability_projectile_spawn(
    trigger: On<Remove, AbilityBulletOf>,
    bullet_query: Query<&AbilityBulletOf>,
    mut commands: Commands,
) {
    if let Ok(bullet_of) = bullet_query.get(trigger.entity) {
        if let Ok(mut c) = commands.get_entity(bullet_of.0) {
            c.prediction_despawn();
        }
    }
}
