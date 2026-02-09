use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_common_assets::ron::RonAssetPlugin;
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::{
    ControlledBy, DisableRollback, LocalTimeline, NetworkTarget, NetworkTimeline, PreSpawned,
    PredictionTarget, Replicate, Tick,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{CharacterMarker, PlayerActions};

/// String-based ability identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct AbilityId(pub String);

/// What an ability does when it activates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum AbilityEffect {
    Melee,
    Projectile { speed: f32, lifetime_ticks: u16 },
    Dash { speed: f32 },
}

/// Definition of a single ability, loaded from RON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub struct AbilityDef {
    pub startup_ticks: u16,
    pub active_ticks: u16,
    pub recovery_ticks: u16,
    pub cooldown_ticks: u16,
    pub steps: u8,
    pub step_window_ticks: u16,
    pub effect: AbilityEffect,
}

/// Asset file containing all ability definitions.
#[derive(Clone, Debug, Serialize, Deserialize, Asset, TypePath)]
#[type_path = "protocol::ability"]
pub struct AbilityDefsAsset {
    pub abilities: HashMap<String, AbilityDef>,
}

/// Resource holding loaded ability definitions, keyed by `AbilityId`.
#[derive(Resource, Clone, Debug)]
pub struct AbilityDefs {
    pub abilities: HashMap<AbilityId, AbilityDef>,
}

impl AbilityDefs {
    pub fn get(&self, id: &AbilityId) -> Option<&AbilityDef> {
        self.abilities.get(id)
    }
}

/// Per-character ability loadout (up to 4 slots).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AbilitySlots(pub [Option<AbilityId>; 4]);

impl Default for AbilitySlots {
    fn default() -> Self {
        Self([None, None, None, None])
    }
}

/// Which phase of an ability is currently executing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Reflect)]
pub enum AbilityPhase {
    Startup,
    Active,
    Recovery,
}

/// Tracks the currently executing ability on a character.
/// Present only while an ability is active; removed when ability completes.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveAbility {
    pub ability_id: AbilityId,
    pub phase: AbilityPhase,
    pub phase_start_tick: Tick,
    /// Current step in a multi-hit combo (0-indexed).
    pub step: u8,
    pub total_steps: u8,
    /// Whether the player pressed the key again during this step's window.
    pub chain_input_received: bool,
}

/// Per-slot cooldown tracking.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AbilityCooldowns {
    pub last_used: [Option<Tick>; 4],
}

impl Default for AbilityCooldowns {
    fn default() -> Self {
        Self {
            last_used: [None; 4],
        }
    }
}

/// Marker on a ProjectileSpawn entity — stores spawn parameters.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub struct AbilityProjectileSpawn {
    pub spawn_tick: Tick,
    pub position: Vec3,
    pub direction: Vec3,
    pub speed: f32,
    pub lifetime_ticks: u16,
    pub ability_id: AbilityId,
    pub shooter: Entity,
}

/// Relationship: projectile belongs to a character.
#[derive(Component, Debug)]
#[relationship(relationship_target = AbilityBullets)]
pub struct AbilityBulletOf(#[entities] pub Entity);

/// Relationship target: character's active projectiles.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = AbilityBulletOf, linked_spawn)]
pub struct AbilityBullets(Vec<Entity>);

// -- Asset loading --

#[derive(Resource)]
struct AbilityDefsHandle(Handle<AbilityDefsAsset>);

pub struct AbilityPlugin;

impl Plugin for AbilityPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RonAssetPlugin::<AbilityDefsAsset>::new(&["abilities.ron"]));
        app.add_systems(Startup, load_ability_defs);
        app.add_systems(Update, insert_ability_defs);
    }
}

fn load_ability_defs(mut commands: Commands, asset_server: Res<AssetServer>) {
    let handle = asset_server.load::<AbilityDefsAsset>("abilities.ron");
    commands.insert_resource(AbilityDefsHandle(handle));
}

fn insert_ability_defs(
    mut commands: Commands,
    handle: Option<Res<AbilityDefsHandle>>,
    assets: Res<Assets<AbilityDefsAsset>>,
    existing: Option<Res<AbilityDefs>>,
) {
    if existing.is_some() {
        return;
    }
    let Some(handle) = handle else { return };
    let Some(asset) = assets.get(&handle.0) else {
        return;
    };
    let abilities: HashMap<AbilityId, AbilityDef> = asset
        .abilities
        .iter()
        .map(|(k, v)| (AbilityId(k.clone()), v.clone()))
        .collect();
    info!("Loaded {} ability definitions", abilities.len());
    commands.insert_resource(AbilityDefs { abilities });
}

/// Maps a `PlayerActions` ability variant to a slot index (0-3).
pub fn ability_action_to_slot(action: &PlayerActions) -> Option<usize> {
    match action {
        PlayerActions::Ability1 => Some(0),
        PlayerActions::Ability2 => Some(1),
        PlayerActions::Ability3 => Some(2),
        PlayerActions::Ability4 => Some(3),
        _ => None,
    }
}

/// Activate an ability when a hotkey is pressed and no ability is currently active.
pub fn ability_activation(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut query: Query<
        (
            Entity,
            &ActionState<PlayerActions>,
            &AbilitySlots,
            &mut AbilityCooldowns,
        ),
        Without<ActiveAbility>,
    >,
) {
    let tick = timeline.tick();

    for (entity, action_state, slots, mut cooldowns) in &mut query {
        for action in [
            PlayerActions::Ability1,
            PlayerActions::Ability2,
            PlayerActions::Ability3,
            PlayerActions::Ability4,
        ] {
            if !action_state.just_pressed(&action) {
                continue;
            }
            let Some(slot_idx) = ability_action_to_slot(&action) else {
                continue;
            };
            let Some(ref ability_id) = slots.0[slot_idx] else {
                continue;
            };
            let Some(def) = ability_defs.get(ability_id) else {
                continue;
            };

            // Check cooldown
            if let Some(last_used) = cooldowns.last_used[slot_idx] {
                let elapsed = tick - last_used;
                if elapsed.unsigned_abs() <= def.cooldown_ticks {
                    continue;
                }
            }

            // Activate
            cooldowns.last_used[slot_idx] = Some(tick);
            commands.entity(entity).insert(ActiveAbility {
                ability_id: ability_id.clone(),
                phase: AbilityPhase::Startup,
                phase_start_tick: tick,
                step: 0,
                total_steps: def.steps,
                chain_input_received: false,
            });
            info!("Ability activated: {:?} (slot {})", ability_id, slot_idx);
            break; // only one ability at a time
        }
    }
}

/// Advance ability phases based on tick counts. Handle multi-step combo chaining.
pub fn ability_phase_advance(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut query: Query<(
        Entity,
        &mut ActiveAbility,
        &ActionState<PlayerActions>,
        &AbilitySlots,
    )>,
) {
    let tick = timeline.tick();

    for (entity, mut active, action_state, slots) in &mut query {
        let Some(def) = ability_defs.get(&active.ability_id) else {
            warn!("Ability {:?} not found", active.ability_id);
            commands.entity(entity).remove::<ActiveAbility>();
            continue;
        };

        let elapsed = tick - active.phase_start_tick;

        // Check for combo chain input (re-pressing the same ability key)
        if !active.chain_input_received && active.step + 1 < active.total_steps {
            for (slot_idx, slot) in slots.0.iter().enumerate() {
                if slot.as_ref() == Some(&active.ability_id) {
                    let action = match slot_idx {
                        0 => PlayerActions::Ability1,
                        1 => PlayerActions::Ability2,
                        2 => PlayerActions::Ability3,
                        3 => PlayerActions::Ability4,
                        _ => continue,
                    };
                    if action_state.just_pressed(&action) {
                        active.chain_input_received = true;
                    }
                    break;
                }
            }
        }

        match active.phase {
            AbilityPhase::Startup => {
                if elapsed >= def.startup_ticks as i16 {
                    active.phase = AbilityPhase::Active;
                    active.phase_start_tick = tick;
                    info!(
                        "Ability {:?} step {} -> Active",
                        active.ability_id, active.step
                    );
                }
            }
            AbilityPhase::Active => {
                if elapsed >= def.active_ticks as i16 {
                    active.phase = AbilityPhase::Recovery;
                    active.phase_start_tick = tick;
                    info!(
                        "Ability {:?} step {} -> Recovery",
                        active.ability_id, active.step
                    );
                }
            }
            AbilityPhase::Recovery => {
                if elapsed >= def.recovery_ticks as i16 {
                    // Check for combo chain
                    if active.chain_input_received && active.step + 1 < active.total_steps {
                        active.step += 1;
                        active.phase = AbilityPhase::Startup;
                        active.phase_start_tick = tick;
                        active.chain_input_received = false;
                        info!("Ability {:?} -> step {}", active.ability_id, active.step);
                    } else {
                        info!("Ability {:?} complete", active.ability_id);
                        commands.entity(entity).remove::<ActiveAbility>();
                    }
                } else if !active.chain_input_received
                    && active.step + 1 < active.total_steps
                    && elapsed >= def.step_window_ticks as i16
                {
                    // Window expired without chain input — end ability
                    info!("Ability {:?} chain window expired", active.ability_id);
                    commands.entity(entity).remove::<ActiveAbility>();
                }
            }
        }
    }
}

/// Spawn a `AbilityProjectileSpawn` entity with `PreSpawned` when a projectile ability enters Active phase.
pub fn ability_projectile_spawn(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    query: Query<(Entity, &ActiveAbility, &Position, &Rotation), With<CharacterMarker>>,
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();

    for (entity, active, position, rotation) in &query {
        if active.phase != AbilityPhase::Active || active.phase_start_tick != tick {
            continue;
        }
        let Some(def) = ability_defs.get(&active.ability_id) else {
            continue;
        };
        let AbilityEffect::Projectile {
            speed,
            lifetime_ticks,
        } = &def.effect
        else {
            continue;
        };

        let direction = (rotation.0 * Vec3::NEG_Z).normalize();
        let spawn_info = AbilityProjectileSpawn {
            spawn_tick: tick,
            position: position.0 + direction * 1.5,
            direction,
            speed: *speed,
            lifetime_ticks: *lifetime_ticks,
            ability_id: active.ability_id.clone(),
            shooter: entity,
        };

        let mut cmd = commands.spawn((
            spawn_info,
            PreSpawned::default_with_salt(active.step as u64),
            Name::new("AbilityProjectileSpawn"),
        ));

        if let Ok(controlled_by) = server_query.get(entity) {
            cmd.insert((
                Replicate::to_clients(NetworkTarget::All),
                PredictionTarget::to_clients(NetworkTarget::All),
                *controlled_by,
            ));
        }
    }
}

/// Spawn child bullet entities from `AbilityProjectileSpawn` parents.
pub fn handle_ability_projectile_spawn(
    mut commands: Commands,
    spawn_query: Query<(Entity, &AbilityProjectileSpawn), Without<AbilityBullets>>,
) {
    for (spawn_entity, spawn_info) in &spawn_query {
        info!("Spawning ability bullet from {:?}", spawn_info.ability_id);
        commands.spawn((
            Position(spawn_info.position),
            Rotation::default(),
            LinearVelocity(spawn_info.direction * spawn_info.speed),
            RigidBody::Kinematic,
            Collider::sphere(0.25),
            AbilityBulletOf(spawn_entity),
            DisableRollback,
            Name::new("AbilityBullet"),
        ));
    }
}

/// When a child bullet's `AbilityBulletOf` is removed, despawn the parent spawn entity.
pub fn despawn_ability_projectile_spawn(
    trigger: On<Remove, AbilityBulletOf>,
    bullet_query: Query<&AbilityBulletOf>,
    mut commands: Commands,
) {
    if let Ok(bullet_of) = bullet_query.get(trigger.entity) {
        if let Ok(mut c) = commands.get_entity(bullet_of.0) {
            c.try_despawn();
        }
    }
}

/// Despawn bullets whose lifetime has expired.
pub fn ability_bullet_lifetime(
    mut commands: Commands,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
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

/// Apply dash velocity while a dash ability is in Active phase.
pub fn ability_dash_effect(
    ability_defs: Res<AbilityDefs>,
    mut query: Query<(&ActiveAbility, &Rotation, &mut LinearVelocity), With<CharacterMarker>>,
) {
    for (active, rotation, mut velocity) in &mut query {
        if active.phase != AbilityPhase::Active {
            continue;
        }
        let Some(def) = ability_defs.get(&active.ability_id) else {
            continue;
        };
        let AbilityEffect::Dash { speed } = &def.effect else {
            continue;
        };

        let direction = (rotation.0 * Vec3::NEG_Z).normalize();
        velocity.x = direction.x * *speed;
        velocity.z = direction.z * *speed;
    }
}
