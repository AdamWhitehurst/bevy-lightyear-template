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

const PROJECTILE_SPAWN_OFFSET: f32 = 1.5;
const BULLET_COLLIDER_RADIUS: f32 = 0.25;

const ABILITY_ACTIONS: [PlayerActions; 4] = [
    PlayerActions::Ability1,
    PlayerActions::Ability2,
    PlayerActions::Ability3,
    PlayerActions::Ability4,
];

pub fn facing_direction(rotation: &Rotation) -> Vec3 {
    (rotation.0 * Vec3::NEG_Z).normalize()
}

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

impl AbilityDef {
    pub fn phase_duration(&self, phase: &AbilityPhase) -> u16 {
        match phase {
            AbilityPhase::Startup => self.startup_ticks,
            AbilityPhase::Active => self.active_ticks,
            AbilityPhase::Recovery => self.recovery_ticks,
        }
    }
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

impl ActiveAbility {
    pub fn has_more_steps(&self) -> bool {
        self.step + 1 < self.total_steps
    }
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

impl AbilityCooldowns {
    pub fn is_on_cooldown(&self, slot: usize, current_tick: Tick, cooldown_ticks: u16) -> bool {
        self.last_used[slot]
            .map(|last| (current_tick - last).unsigned_abs() <= cooldown_ticks)
            .unwrap_or(false)
    }
}

/// Present during Active phase of a Dash ability. Removed on phase exit.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct DashAbilityEffect {
    pub speed: f32,
}

/// One-shot: inserted when Projectile enters Active. Consumed by spawn system.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ProjectileSpawnAbilityEffect {
    pub speed: f32,
    pub lifetime_ticks: u16,
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
    ABILITY_ACTIONS.iter().position(|a| a == action)
}

/// Maps a slot index (0-3) to its corresponding `PlayerActions` variant.
pub fn slot_to_ability_action(slot: usize) -> Option<PlayerActions> {
    ABILITY_ACTIONS.get(slot).copied()
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
        for (slot_idx, action) in ABILITY_ACTIONS.iter().enumerate() {
            if !action_state.just_pressed(action) {
                continue;
            }
            let Some(ref ability_id) = slots.0[slot_idx] else {
                continue;
            };
            let Some(def) = ability_defs.get(ability_id) else {
                continue;
            };
            if cooldowns.is_on_cooldown(slot_idx, tick, def.cooldown_ticks) {
                continue;
            }

            cooldowns.last_used[slot_idx] = Some(tick);
            commands.entity(entity).insert(ActiveAbility {
                ability_id: ability_id.clone(),
                phase: AbilityPhase::Startup,
                phase_start_tick: tick,
                step: 0,
                total_steps: def.steps,
                chain_input_received: false,
            });
            break; // only one ability at a time
        }
    }
}

/// Set `chain_input_received` to true if the player re-pressed the ability key for combo chaining.
fn set_chain_input_received(
    active: &mut ActiveAbility,
    action_state: &ActionState<PlayerActions>,
    slots: &AbilitySlots,
) {
    if active.chain_input_received || !active.has_more_steps() {
        return;
    }
    let Some((slot_idx, _)) = slots
        .0
        .iter()
        .enumerate()
        .find(|(_, slot)| slot.as_ref() == Some(&active.ability_id))
    else {
        return;
    };
    let Some(action) = slot_to_ability_action(slot_idx) else {
        return;
    };
    if action_state.just_pressed(&action) {
        active.chain_input_received = true;
    }
}

/// Advance the ability phase
fn advance_ability_phase(
    commands: &mut Commands,
    entity: Entity,
    active: &mut ActiveAbility,
    def: &AbilityDef,
    tick: Tick,
) {
    let elapsed = tick - active.phase_start_tick;
    let phase_complete = elapsed >= def.phase_duration(&active.phase) as i16;

    match active.phase {
        AbilityPhase::Startup if phase_complete => {
            active.phase = AbilityPhase::Active;
            active.phase_start_tick = tick;
        }
        AbilityPhase::Active if phase_complete => {
            active.phase = AbilityPhase::Recovery;
            active.phase_start_tick = tick;
        }
        AbilityPhase::Recovery if phase_complete => {
            if active.chain_input_received && active.has_more_steps() {
                active.step += 1;
                active.phase = AbilityPhase::Startup;
                active.phase_start_tick = tick;
                active.chain_input_received = false;
            } else {
                commands.entity(entity).remove::<ActiveAbility>();
            }
        }
        AbilityPhase::Recovery => {
            let chain_window_expired = !active.chain_input_received
                && active.has_more_steps()
                && elapsed >= def.step_window_ticks as i16;
            if chain_window_expired {
                commands.entity(entity).remove::<ActiveAbility>();
            }
        }
        _ => {}
    }
}

/// Advance ability phases based on tick counts. Handle multi-step combo chaining.
pub fn update_active_abilities(
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

        set_chain_input_received(&mut active, action_state, slots);
        advance_ability_phase(&mut commands, entity, &mut active, def, tick);
    }
}

/// Insert/remove effect marker components based on `ActiveAbility` phase.
/// Centralizes the `AbilityDefs` lookup so effect systems query markers directly.
pub fn dispatch_effect_markers(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    query: Query<(Entity, &ActiveAbility)>,
) {
    let tick = timeline.tick();

    for (entity, active) in &query {
        let Some(def) = ability_defs.get(&active.ability_id) else {
            warn!("dispatch_effect_markers: ability {:?} not found", active.ability_id);
            continue;
        };

        if active.phase == AbilityPhase::Active {
            dispatch_while_active_markers(&mut commands, entity, def);
            if active.phase_start_tick == tick {
                dispatch_on_cast_markers(&mut commands, entity, def);
            }
        } else {
            remove_while_active_markers(&mut commands, entity);
        }
    }
}

fn dispatch_while_active_markers(commands: &mut Commands, entity: Entity, def: &AbilityDef) {
    if let AbilityEffect::Dash { speed } = &def.effect {
        commands
            .entity(entity)
            .insert(DashAbilityEffect { speed: *speed });
    }
}

fn dispatch_on_cast_markers(commands: &mut Commands, entity: Entity, def: &AbilityDef) {
    if let AbilityEffect::Projectile {
        speed,
        lifetime_ticks,
    } = &def.effect
    {
        commands
            .entity(entity)
            .insert(ProjectileSpawnAbilityEffect {
                speed: *speed,
                lifetime_ticks: *lifetime_ticks,
            });
    }
}

fn remove_while_active_markers(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).remove::<DashAbilityEffect>();
}

/// Safety net: remove all effect markers when `ActiveAbility` is removed.
pub fn cleanup_effect_markers_on_removal(
    trigger: On<Remove, ActiveAbility>,
    mut commands: Commands,
) {
    if let Ok(mut cmd) = commands.get_entity(trigger.entity) {
        cmd.remove::<DashAbilityEffect>();
        cmd.remove::<ProjectileSpawnAbilityEffect>();
    }
}

/// Spawn a `AbilityProjectileSpawn` entity from `ProjectileSpawnAbilityEffect` markers.
pub fn ability_projectile_spawn(
    mut commands: Commands,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    query: Query<
        (
            Entity,
            &ProjectileSpawnAbilityEffect,
            &ActiveAbility,
            &Position,
            &Rotation,
        ),
        With<CharacterMarker>,
    >,
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();

    for (entity, request, active, position, rotation) in &query {
        let direction = facing_direction(rotation);
        let spawn_info = AbilityProjectileSpawn {
            spawn_tick: tick,
            position: position.0 + direction * PROJECTILE_SPAWN_OFFSET,
            direction,
            speed: request.speed,
            lifetime_ticks: request.lifetime_ticks,
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

        commands
            .entity(entity)
            .remove::<ProjectileSpawnAbilityEffect>();
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
            Collider::sphere(BULLET_COLLIDER_RADIUS),
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

/// Apply dash velocity while `DashAbilityEffect` marker is present.
pub fn ability_dash_effect(
    mut query: Query<(&DashAbilityEffect, &Rotation, &mut LinearVelocity), With<CharacterMarker>>,
) {
    for (dash, rotation, mut velocity) in &mut query {
        let direction = facing_direction(rotation);
        velocity.x = direction.x * dash.speed;
        velocity.z = direction.z * dash.speed;
    }
}
