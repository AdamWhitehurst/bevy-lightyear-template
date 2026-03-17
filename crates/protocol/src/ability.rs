use avian3d::prelude::*;
use bevy::asset::io::Reader;
use bevy::asset::AssetPath;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::reflect::{PartialReflect, TypeRegistryArc};
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::PredictionDespawnCommandsExt;
use lightyear::prelude::{
    ControlledBy, DisableRollback, LocalTimeline, NetworkTarget, PreSpawned, PredictionTarget,
    Replicate, Replicated, Tick,
};
use lightyear::utils::collections::EntityHashSet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

#[cfg(not(target_arch = "wasm32"))]
use bevy::asset::LoadedFolder;

use crate::hit_detection::{
    hitbox_collision_layers, MELEE_HITBOX_HALF_EXTENTS, MELEE_HITBOX_OFFSET,
};
use crate::map::MapInstanceId;
use crate::{PlayerActions, PlayerId};

const PROJECTILE_SPAWN_OFFSET: f32 = 3.0;
const BULLET_COLLIDER_RADIUS: f32 = 0.5;

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

/// Specifies who receives an effect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect, Default)]
pub enum EffectTarget {
    #[default]
    Caster,
    Victim,
    OriginalCaster,
}

/// Coordinate frame used to interpret a force vector in [`AbilityEffect::ApplyForce`].
///
/// Given `force = Vec3::new(0.0, -10.0, 10.0)` ("back and up"):
///
/// - [`World`]: applied as `(0, -10, 10)` in global space. Neither body's rotation matters.
/// - [`Caster`]: `caster_rotation * (0, -10, 10)`. "Away from where the caster faces, upward
///   relative to the caster." Rotates with the caster; victim orientation is irrelevant.
/// - [`Victim`]: `victim_rotation * (0, -10, 10)`. "Backward and up from the victim's own
///   perspective." Rotates with the victim; caster orientation is irrelevant.
/// - [`RelativePosition`]: frame built from the caster→victim displacement. +Z points toward
///   the victim, +Y is world up, +X is the right-hand cross product. Useful for "push target
///   away from me" regardless of either body's facing direction.
/// - [`RelativeRotation`]: `(victim_rotation * caster_rotation.inverse()) * force`. Captures
///   how rotationally misaligned the two bodies are; the result changes only when their
///   relative orientation changes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Reflect)]
pub enum ForceFrame {
    /// Force in global (world) space.
    #[default]
    World,
    /// Force in the caster's local space — rotates with the caster.
    Caster,
    /// Force in the victim's local space — rotates with the victim.
    Victim,
    /// Force in a frame where +Z is the caster-to-victim direction and +Y is world up.
    RelativePosition,
    /// Force scaled by `victim_rotation * caster_rotation.inverse()`.
    RelativeRotation,
}

/// What an ability does when it activates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum AbilityEffect {
    Melee {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        target: EffectTarget,
    },
    Projectile {
        #[serde(default)]
        id: Option<String>,
        speed: f32,
        lifetime_ticks: u16,
    },
    SetVelocity {
        speed: f32,
        target: EffectTarget,
    },
    Damage {
        amount: f32,
        target: EffectTarget,
    },
    ApplyForce {
        force: Vec3,
        #[serde(default)]
        frame: ForceFrame,
        target: EffectTarget,
    },
    AreaOfEffect {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        target: EffectTarget,
        radius: f32,
        /// How many ticks the AoE hitbox persists. `None` defaults to 1 tick.
        #[serde(default)]
        duration_ticks: Option<u16>,
    },
    /// Spawn a sub-ability as an independent `ActiveAbility` entity.
    ///
    /// The sub-ability goes through its own full phase cycle (Startup → Active → Recovery).
    /// **Latency**: adds at minimum 1 tick before the sub-ability's effects fire, because
    /// the spawned entity is created via `Commands` and won't be processed by
    /// `update_active_abilities` until the next tick.
    ///
    /// For same-tick sequencing, use multiple `OnTick` effects on the parent ability instead.
    /// Reserve `Ability` for when you need independent phase cycles or different `OnHit` effects.
    Ability {
        id: String,
        target: EffectTarget,
    },
    /// Instantly move caster forward by `distance` units in facing direction.
    Teleport {
        distance: f32,
    },
    /// Grant a damage-absorbing shield to the caster.
    Shield {
        absorb: f32,
    },
    /// Apply a temporary stat multiplier to the target.
    Buff {
        stat: String,
        multiplier: f32,
        duration_ticks: u16,
        target: EffectTarget,
    },
}

/// Controls when an effect fires during an ability's lifecycle.
///
/// Effects that need to fire on specific ticks within the Active phase use `OnTick`
/// with a tick offset. Multiple `OnTick` effects on the same ability fire on the same
/// tick if they share the same offset — use different offsets to sequence them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum EffectTrigger {
    /// Fires once on the specified Active-phase tick offset (0-indexed from phase start).
    /// Defaults to tick 0 (first Active tick) when `tick` is omitted in RON.
    OnTick {
        #[serde(default)]
        tick: u16,
        effect: AbilityEffect,
    },
    /// Fires every tick during Active phase.
    WhileActive(AbilityEffect),
    /// Fires when a hitbox/projectile spawned by this ability hits a target.
    OnHit(AbilityEffect),
    /// Fires once when ability exits Active phase (enters Recovery).
    OnEnd(AbilityEffect),
    /// Fires during Active phase when the specified input is just-pressed.
    OnInput {
        action: PlayerActions,
        effect: AbilityEffect,
    },
}

/// Definition of a single ability, loaded from an individual `.ability.ron` file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect, Asset)]
pub struct AbilityDef {
    pub startup_ticks: u16,
    pub active_ticks: u16,
    pub recovery_ticks: u16,
    pub cooldown_ticks: u16,
    pub effects: Vec<EffectTrigger>,
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

/// Tick-based phase durations and cooldown. Loaded from RON archetype.
#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct AbilityPhases {
    pub startup: u16,
    pub active: u16,
    pub recovery: u16,
    pub cooldown: u16,
}

impl AbilityPhases {
    pub fn phase_duration(&self, phase: &AbilityPhase) -> u16 {
        match phase {
            AbilityPhase::Startup => self.startup,
            AbilityPhase::Active => self.active,
            AbilityPhase::Recovery => self.recovery,
        }
    }
}

/// Manifest listing ability IDs, used by WASM builds where `load_folder` is unavailable.
#[derive(Clone, Debug, Serialize, Deserialize, Asset, TypePath)]
pub struct AbilityManifest(pub Vec<String>);

/// Resource holding loaded ability asset handles, keyed by `AbilityId`.
#[derive(Resource, Clone, Debug, Default)]
pub struct AbilityDefs {
    pub abilities: HashMap<AbilityId, Handle<AbilityAsset>>,
}

impl AbilityDefs {
    pub fn get(&self, id: &AbilityId) -> Option<&Handle<AbilityAsset>> {
        self.abilities.get(id)
    }
}

/// Per-character ability loadout (up to 4 slots).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, Asset, TypePath)]
#[type_path = "protocol::ability"]
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

/// Tracks an executing ability as a standalone predicted entity.
/// Spawned when ability activates; despawned when ability completes.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveAbility {
    pub def_id: AbilityId,
    pub caster: Entity,
    pub original_caster: Entity,
    pub target: Entity,
    pub phase: AbilityPhase,
    pub phase_start_tick: Tick,
    pub ability_slot: u8,
    pub depth: u8,
}

impl MapEntities for ActiveAbility {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.caster = entity_mapper.get_mapped(self.caster);
        self.original_caster = entity_mapper.get_mapped(self.original_caster);
        self.target = entity_mapper.get_mapped(self.target);
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

/// One-shot: inserted by apply_on_tick_effects when processing Projectile.
/// Consumed by ability_projectile_spawn.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ProjectileSpawnEffect {
    pub speed: f32,
    pub lifetime_ticks: u16,
}

/// Relationship: hitbox entity belongs to an ActiveAbility entity.
#[derive(Component, Debug)]
#[relationship(relationship_target = ActiveAbilityHitboxes)]
pub struct HitboxOf(#[entities] pub Entity);

/// Relationship target: ActiveAbility's spawned hitbox entities.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = HitboxOf, linked_spawn)]
pub struct ActiveAbilityHitboxes(Vec<Entity>);

/// Marker on hitbox entities that need to track caster position each tick.
#[derive(Component, Clone, Debug)]
pub struct MeleeHitbox;

/// Tracks spawn tick and duration for AoE hitbox lifetime management.
#[derive(Component, Clone, Debug)]
pub struct AoEHitbox {
    pub spawn_tick: Tick,
    pub duration_ticks: u16,
}

/// Tracks entities already hit by this hitbox to prevent duplicate effects.
#[derive(Component, Clone, Debug, Default)]
pub struct HitTargets(pub EntityHashSet);

/// Carried on ActiveAbility entities (for melee) and bullet entities (for projectiles).
/// Hit detection systems read this to determine what effects to apply on contact.
#[derive(Component, Clone, Debug)]
pub struct OnHitEffects {
    pub effects: Vec<AbilityEffect>,
    pub caster: Entity,
    pub original_caster: Entity,
    pub depth: u8,
}

/// Active-phase tick effect with offset metadata.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct TickEffect {
    /// Active-phase tick offset. Defaults to 0.
    #[serde(default)]
    pub tick: u16,
    pub effect: AbilityEffect,
}

/// Archetype component: all tick-triggered effects with their offsets.
/// Persists on the ActiveAbility entity for the ability's lifetime.
/// Apply systems filter by current tick offset directly — no intermediate dispatch component.
#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct OnTickEffects(pub Vec<TickEffect>);

/// Archetype component: effects that fire every tick during Active phase.
#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct WhileActiveEffects(pub Vec<AbilityEffect>);

/// Archetype component: effects that fire when Active → Recovery.
#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct OnEndEffects(pub Vec<AbilityEffect>);

/// Input-triggered effect with action metadata.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct InputEffect {
    pub action: PlayerActions,
    pub effect: AbilityEffect,
}

/// Archetype component: input-triggered effects during Active phase.
#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct OnInputEffects(pub Vec<InputEffect>);

/// Archetype component: effects applied when a hitbox/projectile hits a target.
/// Does NOT contain caster/depth — those come from ActiveAbility at dispatch time.
#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct OnHitEffectDefs(pub Vec<AbilityEffect>);

/// A bundle of reflected components loaded from a `.ability.ron` file.
/// Replaces `AbilityDef` as the asset type.
#[derive(Asset, TypePath)]
pub struct AbilityAsset {
    pub components: Vec<Box<dyn PartialReflect>>,
}

impl Clone for AbilityAsset {
    fn clone(&self) -> Self {
        Self {
            components: self
                .components
                .iter()
                .map(|c| {
                    c.reflect_clone()
                        .expect("ability component must be cloneable")
                        .into_partial_reflect()
                })
                .collect(),
        }
    }
}

impl fmt::Debug for AbilityAsset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AbilityAsset")
            .field(
                "components",
                &self
                    .components
                    .iter()
                    .map(|c| c.reflect_type_path())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Extract AbilityPhases from an AbilityAsset's reflected components.
pub fn extract_phases(asset: &AbilityAsset) -> Option<&AbilityPhases> {
    let target_id = std::any::TypeId::of::<AbilityPhases>();
    for reflected in &asset.components {
        let info = reflected
            .get_represented_type_info()
            .expect("AbilityAsset should have type info");

        if info.type_id() == target_id {
            return reflected.try_downcast_ref::<AbilityPhases>();
        }
    }
    None
}

/// Insert all reflected components from an `AbilityAsset` onto an entity.
fn apply_ability_archetype(
    commands: &mut Commands,
    entity: Entity,
    asset: &AbilityAsset,
    registry: TypeRegistryArc,
) {
    let components: Vec<Box<dyn PartialReflect>> = asset
        .components
        .iter()
        .map(|c| {
            c.reflect_clone()
                .expect("ability component must be cloneable")
                .into_partial_reflect()
        })
        .collect();

    commands.queue(move |world: &mut World| {
        let registry = registry.read();
        let mut entity_mut = world.entity_mut(entity);
        for component in &components {
            let type_path = component.reflect_type_path();
            let Some(registration) = registry.get_with_type_path(type_path) else {
                warn!("Ability component type not registered: {type_path}");
                continue;
            };
            let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                warn!("Type missing #[reflect(Component)]: {type_path}");
                continue;
            };
            reflect_component.insert(&mut entity_mut, component.as_ref(), &registry);
        }
    });
}

/// Custom asset loader for `.ability.ron` files using reflect-based deserialization.
#[derive(TypePath)]
struct AbilityAssetLoader {
    type_registry: TypeRegistryArc,
}

impl FromWorld for AbilityAssetLoader {
    fn from_world(world: &mut World) -> Self {
        Self {
            type_registry: world.resource::<AppTypeRegistry>().0.clone(),
        }
    }
}

impl AssetLoader for AbilityAssetLoader {
    type Asset = AbilityAsset;
    type Settings = ();
    type Error = crate::reflect_loader::ReflectLoadError;

    fn extensions(&self) -> &[&str] {
        &["ability.ron"]
    }

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let registry = self.type_registry.read();
        let components = crate::reflect_loader::deserialize_component_map(&bytes, &registry)?;
        Ok(AbilityAsset { components })
    }
}

/// Damage absorption shield on a character. Intercepts damage before Health.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveShield {
    pub remaining: f32,
}

/// Temporary stat modifiers on a character. Tick-based expiry.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveBuffs(pub Vec<ActiveBuff>);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveBuff {
    pub stat: String,
    pub multiplier: f32,
    pub expires_tick: Tick,
}

/// Marker on a ProjectileSpawn entity -- stores spawn parameters.
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

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
struct AbilityFolderHandle(Handle<LoadedFolder>);

#[cfg(target_arch = "wasm32")]
#[derive(Resource)]
struct AbilityManifestHandle(Handle<AbilityManifest>);

#[cfg(target_arch = "wasm32")]
#[derive(Resource)]
struct PendingAbilityHandles(Vec<Handle<AbilityAsset>>);

/// Internal handle for the default ability slots asset — used only for loading and hot-reload.
///
/// Note: Separation of DefaultAbilitySlotsHandle and DefaultAbilitySlots enables testing without AssetsPlugin
#[derive(Resource)]
struct DefaultAbilitySlotsHandle(Handle<AbilitySlots>);

/// The resolved global default ability slots, populated once the asset finishes loading.
///
/// Systems read this directly; consumers do not need to touch `Assets<AbilitySlots>`.
///
/// Note: Separation of DefaultAbilitySlotsHandle and DefaultAbilitySlots enables testing without AssetsPlugin
#[derive(Resource, Clone, Default)]
pub struct DefaultAbilitySlots(pub AbilitySlots);

pub struct AbilityPlugin;

impl Plugin for AbilityPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<AbilityPhases>()
            .register_type::<OnTickEffects>()
            .register_type::<TickEffect>()
            .register_type::<WhileActiveEffects>()
            .register_type::<OnHitEffectDefs>()
            .register_type::<OnEndEffects>()
            .register_type::<OnInputEffects>()
            .register_type::<InputEffect>()
            .register_type::<AbilityEffect>()
            .register_type::<EffectTarget>()
            .register_type::<ForceFrame>()
            .register_type::<PlayerActions>();

        app.init_asset::<AbilityAsset>()
            .init_asset_loader::<AbilityAssetLoader>();
        app.add_plugins(
            bevy_common_assets::ron::RonAssetPlugin::<AbilitySlots>::new(&["ability_slots.ron"]),
        );

        #[cfg(target_arch = "wasm32")]
        app.add_plugins(
            bevy_common_assets::ron::RonAssetPlugin::<AbilityManifest>::new(&[
                "abilities.manifest.ron",
            ]),
        );

        app.add_systems(Startup, (load_ability_defs, load_default_ability_slots));

        #[cfg(target_arch = "wasm32")]
        app.add_systems(
            PreUpdate,
            trigger_individual_ability_loads.run_if(in_state(crate::app_state::AppState::Loading)),
        );

        app.add_systems(
            Update,
            (
                insert_ability_defs,
                reload_ability_defs,
                sync_default_ability_slots,
            ),
        );

        let ready = in_state(crate::app_state::AppState::Ready);

        app.add_systems(
            FixedUpdate,
            (
                ability_activation,
                update_active_abilities,
                apply_on_tick_effects,
                apply_while_active_effects,
                apply_on_end_effects,
                apply_on_input_effects,
                ability_projectile_spawn,
            )
                .chain()
                .run_if(ready.clone()),
        );

        app.add_systems(
            FixedUpdate,
            (
                crate::hit_detection::update_hitbox_positions,
                crate::hit_detection::process_hitbox_hits,
                crate::hit_detection::process_projectile_hits,
                crate::hit_detection::cleanup_hitbox_entities,
            )
                .chain()
                .after(apply_on_tick_effects)
                .run_if(ready.clone()),
        );

        app.add_systems(
            FixedUpdate,
            (expire_buffs, aoe_hitbox_lifetime, ability_bullet_lifetime)
                .after(crate::hit_detection::process_hitbox_hits)
                .after(crate::hit_detection::process_projectile_hits)
                .run_if(ready.clone()),
        );
        app.add_systems(PreUpdate, handle_ability_projectile_spawn);
        app.add_observer(despawn_ability_projectile_spawn);
        app.add_observer(cleanup_effect_markers_on_removal);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_ability_defs(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<crate::app_state::TrackedAssets>,
) {
    let handle = asset_server.load_folder("abilities");
    tracked.add(handle.clone());
    commands.insert_resource(AbilityFolderHandle(handle));
}

#[cfg(target_arch = "wasm32")]
fn load_ability_defs(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<crate::app_state::TrackedAssets>,
) {
    let handle = asset_server.load::<AbilityManifest>("abilities.manifest.ron");
    tracked.add(handle.clone());
    commands.insert_resource(AbilityManifestHandle(handle));
}

#[cfg(target_arch = "wasm32")]
fn trigger_individual_ability_loads(
    manifest_handle: Option<Res<AbilityManifestHandle>>,
    manifest_assets: Res<Assets<AbilityManifest>>,
    pending: Option<Res<PendingAbilityHandles>>,
    mut tracked: ResMut<crate::app_state::TrackedAssets>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    if pending.is_some() {
        trace!("PendingAbilityHandles already exists");
        return;
    }
    let Some(manifest_handle) = manifest_handle else {
        trace!("ability manifest handle not yet loaded");
        return;
    };
    let Some(manifest) = manifest_assets.get(&manifest_handle.0) else {
        trace!("ability manifest asset not yet available");
        return;
    };
    let handles: Vec<Handle<AbilityAsset>> = manifest
        .0
        .iter()
        .map(|id| {
            let h: Handle<AbilityAsset> = asset_server.load(format!("abilities/{id}.ability.ron"));
            tracked.add(h.clone());
            h
        })
        .collect();
    commands.insert_resource(PendingAbilityHandles(handles));
}

#[cfg(not(target_arch = "wasm32"))]
fn insert_ability_defs(
    mut commands: Commands,
    folder_handle: Option<Res<AbilityFolderHandle>>,
    loaded_folders: Res<Assets<LoadedFolder>>,
    asset_server: Res<AssetServer>,
    existing: Option<Res<AbilityDefs>>,
) {
    if existing.is_some() {
        trace!("AbilityDefs already inserted");
        return;
    }
    let Some(folder_handle) = folder_handle else {
        trace!("ability folder handle not yet loaded");
        return;
    };
    let Some(folder) = loaded_folders.get(&folder_handle.0) else {
        trace!("ability folder not yet available in Assets<LoadedFolder>");
        return;
    };
    let abilities = collect_ability_handles_from_folder(folder, &asset_server);
    info!("Loaded {} ability definitions", abilities.len());
    commands.insert_resource(AbilityDefs { abilities });
}

#[cfg(target_arch = "wasm32")]
fn insert_ability_defs(
    mut commands: Commands,
    pending: Option<Res<PendingAbilityHandles>>,
    ability_assets: Res<Assets<AbilityAsset>>,
    asset_server: Res<AssetServer>,
    existing: Option<Res<AbilityDefs>>,
) {
    if existing.is_some() {
        trace!("AbilityDefs already inserted");
        return;
    }
    let Some(pending) = pending else {
        trace!("PendingAbilityHandles not yet available");
        return;
    };
    let abilities: HashMap<AbilityId, Handle<AbilityAsset>> = pending
        .0
        .iter()
        .filter_map(|handle| {
            // Verify asset is loaded before including
            ability_assets.get(handle)?;
            let path = asset_server.get_path(handle.id())?;
            let id = ability_id_from_path(&path)?;
            Some((id, handle.clone()))
        })
        .collect();
    if abilities.len() != pending.0.len() {
        trace!(
            "not all ability assets loaded yet ({}/{})",
            abilities.len(),
            pending.0.len()
        );
        return;
    }
    info!("Loaded {} ability definitions", abilities.len());
    commands.insert_resource(AbilityDefs { abilities });
}

#[cfg(not(target_arch = "wasm32"))]
fn reload_ability_defs(
    mut commands: Commands,
    folder_handle: Option<Res<AbilityFolderHandle>>,
    loaded_folders: Res<Assets<LoadedFolder>>,
    asset_server: Res<AssetServer>,
    mut events: MessageReader<AssetEvent<AbilityAsset>>,
) {
    let Some(folder_handle) = folder_handle else {
        // Folder handle not yet loaded during startup — drain events to avoid stale backlog
        events.clear();
        return;
    };
    let has_changes = events
        .read()
        .any(|e| matches!(e, AssetEvent::Modified { .. }));
    if !has_changes {
        // No ability asset modifications this frame
        return;
    }
    let Some(folder) = loaded_folders.get(&folder_handle.0) else {
        warn!("ability assets changed but LoadedFolder not available");
        return;
    };
    let abilities = collect_ability_handles_from_folder(folder, &asset_server);
    info!("Hot-reloaded {} ability definitions", abilities.len());
    commands.insert_resource(AbilityDefs { abilities });
}

#[cfg(target_arch = "wasm32")]
fn reload_ability_defs(
    mut commands: Commands,
    pending: Option<Res<PendingAbilityHandles>>,
    ability_assets: Res<Assets<AbilityAsset>>,
    asset_server: Res<AssetServer>,
    mut events: MessageReader<AssetEvent<AbilityAsset>>,
) {
    let Some(pending) = pending else {
        // Pending handles not yet available during startup — drain events to avoid stale backlog
        events.clear();
        return;
    };
    let has_changes = events
        .read()
        .any(|e| matches!(e, AssetEvent::Modified { .. }));
    if !has_changes {
        // No ability asset modifications this frame
        return;
    }
    let abilities: HashMap<AbilityId, Handle<AbilityAsset>> = pending
        .0
        .iter()
        .filter_map(|handle| {
            ability_assets.get(handle)?;
            let path = asset_server.get_path(handle.id())?;
            let id = ability_id_from_path(&path)?;
            Some((id, handle.clone()))
        })
        .collect();
    if abilities.len() != pending.0.len() {
        trace!(
            "not all ability assets loaded yet for reload ({}/{})",
            abilities.len(),
            pending.0.len()
        );
        return;
    }
    info!("Hot-reloaded {} ability definitions", abilities.len());
    commands.insert_resource(AbilityDefs { abilities });
}

fn ability_id_from_path(path: &AssetPath) -> Option<AbilityId> {
    let name = path.path().file_name()?.to_str()?;
    Some(AbilityId(name.strip_suffix(".ability.ron")?.to_string()))
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_ability_handles_from_folder(
    folder: &LoadedFolder,
    asset_server: &AssetServer,
) -> HashMap<AbilityId, Handle<AbilityAsset>> {
    folder
        .handles
        .iter()
        .filter_map(|handle| {
            let path = asset_server.get_path(handle.id())?;
            let name = path.path().file_name()?.to_str()?;
            if !name.ends_with(".ability.ron") {
                return None;
            }
            let id = ability_id_from_path(&path)?;
            let typed = handle.clone().typed::<AbilityAsset>();
            Some((id, typed))
        })
        .collect()
}

fn load_default_ability_slots(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<crate::app_state::TrackedAssets>,
) {
    let handle = asset_server.load::<AbilitySlots>("default.ability_slots.ron");
    tracked.add(handle.clone());
    commands.insert_resource(DefaultAbilitySlotsHandle(handle));
}

fn sync_default_ability_slots(
    mut commands: Commands,
    handle: Option<Res<DefaultAbilitySlotsHandle>>,
    ability_slots_assets: Res<Assets<AbilitySlots>>,
    mut events: MessageReader<AssetEvent<AbilitySlots>>,
) {
    let Some(handle) = handle else {
        events.clear();
        return;
    };
    let id = handle.0.id();
    let is_relevant = |e: &AssetEvent<AbilitySlots>| {
        matches!(e,
            AssetEvent::LoadedWithDependencies { id: eid } |
            AssetEvent::Modified { id: eid }
            if *eid == id
        )
    };
    if !events.read().any(is_relevant) {
        return;
    }
    let Some(slots) = ability_slots_assets.get(&handle.0) else {
        warn!("default.ability_slots.ron event fired but asset not available");
        return;
    };
    info!("Synced default ability slots");
    commands.insert_resource(DefaultAbilitySlots(slots.clone()));
}

/// Maps a `PlayerActions` ability variant to a slot index (0-3).
pub fn ability_action_to_slot(action: &PlayerActions) -> Option<usize> {
    ABILITY_ACTIONS.iter().position(|a| a == action)
}

/// Maps a slot index (0-3) to its corresponding `PlayerActions` variant.
pub fn slot_to_ability_action(slot: usize) -> Option<PlayerActions> {
    ABILITY_ACTIONS.get(slot).copied()
}

/// Activate an ability when a hotkey is pressed. Spawns an ActiveAbility entity
/// with all archetype components from the ability asset.
pub fn ability_activation(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    ability_assets: Res<Assets<AbilityAsset>>,
    registry: Res<AppTypeRegistry>,
    default_slots: Res<DefaultAbilitySlots>,
    timeline: Res<LocalTimeline>,
    mut query: Query<(
        Entity,
        &ActionState<PlayerActions>,
        Option<&AbilitySlots>,
        &mut AbilityCooldowns,
        &PlayerId,
    )>,
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();

    for (entity, action_state, slots_opt, mut cooldowns, player_id) in &mut query {
        let slots = slots_opt.unwrap_or(&default_slots.0);
        for (slot_idx, action) in ABILITY_ACTIONS.iter().enumerate() {
            if !action_state.just_pressed(action) {
                continue;
            }
            let Some(ref ability_id) = slots.0[slot_idx] else {
                continue;
            };
            let Some(handle) = ability_defs.get(ability_id) else {
                warn!("Ability {:?} not found in defs", ability_id);
                continue;
            };
            let Some(asset) = ability_assets.get(handle) else {
                warn!("Ability {:?} asset not loaded", ability_id);
                continue;
            };
            let Some(phases) = extract_phases(asset) else {
                warn!("Ability {:?} missing AbilityPhases component", ability_id);
                continue;
            };
            if cooldowns.is_on_cooldown(slot_idx, tick, phases.cooldown) {
                continue;
            }

            cooldowns.last_used[slot_idx] = Some(tick);
            let salt = (player_id.0.to_bits()) << 32 | (slot_idx as u64) << 16 | 0u64;

            let entity_id = commands
                .spawn((
                    ActiveAbility {
                        def_id: ability_id.clone(),
                        caster: entity,
                        original_caster: entity,
                        target: entity,
                        phase: AbilityPhase::Startup,
                        phase_start_tick: tick,
                        ability_slot: slot_idx as u8,
                        depth: 0,
                    },
                    PreSpawned::default_with_salt(salt),
                    Name::new("ActiveAbility"),
                ))
                .id();

            apply_ability_archetype(&mut commands, entity_id, asset, registry.0.clone());

            if let Ok(controlled_by) = server_query.get(entity) {
                commands.entity(entity_id).insert((
                    Replicate::to_clients(NetworkTarget::All),
                    PredictionTarget::to_clients(NetworkTarget::All),
                    *controlled_by,
                ));
            }
        }
    }
}

fn advance_ability_phase(
    commands: &mut Commands,
    entity: Entity,
    active: &mut ActiveAbility,
    phases: &AbilityPhases,
    tick: Tick,
) {
    let elapsed = tick - active.phase_start_tick;
    let phase_complete = elapsed >= phases.phase_duration(&active.phase) as i16;

    if !phase_complete {
        return;
    }

    match active.phase {
        AbilityPhase::Startup => {
            active.phase = AbilityPhase::Active;
            active.phase_start_tick = tick;
        }
        AbilityPhase::Active => {
            active.phase = AbilityPhase::Recovery;
            active.phase_start_tick = tick;
        }
        AbilityPhase::Recovery => {
            commands.entity(entity).prediction_despawn();
        }
    }
}

/// Advance ability phases based on tick counts. Constructs `OnHitEffects` on
/// Startup->Active transition and removes it when leaving Active.
pub fn update_active_abilities(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    mut query: Query<(
        Entity,
        &mut ActiveAbility,
        &AbilityPhases,
        Option<&OnHitEffectDefs>,
    )>,
) {
    let tick = timeline.tick();

    for (entity, mut active, phases, on_hit_defs) in &mut query {
        let prev_phase = active.phase.clone();
        advance_ability_phase(&mut commands, entity, &mut active, phases, tick);

        if active.phase == AbilityPhase::Active && prev_phase != AbilityPhase::Active {
            if let Some(defs) = on_hit_defs {
                if !defs.0.is_empty() {
                    commands.entity(entity).insert(OnHitEffects {
                        effects: defs.0.clone(),
                        caster: active.caster,
                        original_caster: active.original_caster,
                        depth: active.depth,
                    });
                }
            }
        }

        if active.phase != AbilityPhase::Active && prev_phase == AbilityPhase::Active {
            commands.entity(entity).remove::<OnHitEffects>();
        }
    }
}

/// Resolve an EffectTarget to an entity using ActiveAbility's caster fields.
/// Only valid for Caster/OriginalCaster — Victim requires hit context.
fn resolve_caster_target(target: &EffectTarget, active: &ActiveAbility) -> Entity {
    match target {
        EffectTarget::Caster => active.caster,
        EffectTarget::OriginalCaster => active.original_caster,
        other => {
            warn!(
                "EffectTarget::{:?} not valid in caster context, falling back to caster",
                other
            );
            active.caster
        }
    }
}

fn compute_sub_ability_salt(player_id: PlayerId, slot: u8, depth: u8, id: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    player_id.0.to_bits().hash(&mut hasher);
    slot.hash(&mut hasher);
    depth.hash(&mut hasher);
    id.hash(&mut hasher);
    hasher.finish()
}

/// Spawn a sub-ability entity for recursive ability composition.
/// Caps at depth 4 to prevent infinite recursion. Applies archetype components from the asset.
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

    apply_ability_archetype(commands, entity_id, asset, registry.clone());

    if let Ok(controlled_by) = server_query.get(original_caster) {
        commands.entity(entity_id).insert((
            Replicate::to_clients(NetworkTarget::All),
            PredictionTarget::to_clients(NetworkTarget::All),
            *controlled_by,
        ));
    }
}

/// Process OnTick effects: spawn hitbox entities, projectiles, or sub-abilities.
/// Only fires during Active phase; filters by tick offset within the phase.
pub fn apply_on_tick_effects(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    ability_assets: Res<Assets<AbilityAsset>>,
    registry: Res<AppTypeRegistry>,
    timeline: Res<LocalTimeline>,
    server_query: Query<&ControlledBy>,
    player_id_query: Query<&PlayerId>,
    query: Query<(
        Entity,
        &OnTickEffects,
        &ActiveAbility,
        Option<&OnHitEffects>,
    )>,
    mut caster_query: Query<(&mut Position, &Rotation, &MapInstanceId)>,
) {
    let tick = timeline.tick();
    for (entity, effects, active, on_hit_effects) in &query {
        if active.phase != AbilityPhase::Active {
            continue;
        }

        let active_offset = (tick - active.phase_start_tick) as u16;
        for tick_effect in &effects.0 {
            if tick_effect.tick != active_offset {
                continue;
            }
            match &tick_effect.effect {
                AbilityEffect::Melee { .. } => {
                    spawn_melee_hitbox(
                        &mut commands,
                        entity,
                        active,
                        on_hit_effects,
                        &caster_query,
                    );
                }
                AbilityEffect::AreaOfEffect {
                    radius,
                    duration_ticks,
                    ..
                } => {
                    spawn_aoe_hitbox(
                        &mut commands,
                        entity,
                        active,
                        on_hit_effects,
                        &caster_query,
                        *radius,
                        tick,
                        duration_ticks.unwrap_or(1),
                    );
                }
                AbilityEffect::Projectile {
                    speed,
                    lifetime_ticks,
                    ..
                } => {
                    commands.entity(entity).insert(ProjectileSpawnEffect {
                        speed: *speed,
                        lifetime_ticks: *lifetime_ticks,
                    });
                }
                AbilityEffect::Ability { id, target } => {
                    let target_entity = resolve_caster_target(target, active);
                    spawn_sub_ability(
                        &mut commands,
                        ability_defs.as_ref(),
                        ability_assets.as_ref(),
                        &registry.0,
                        id,
                        target_entity,
                        active.original_caster,
                        active.ability_slot,
                        active.depth,
                        tick,
                        &server_query,
                        &player_id_query,
                    );
                }
                AbilityEffect::Teleport { distance } => {
                    apply_teleport(&mut caster_query, active.caster, *distance);
                }
                AbilityEffect::Shield { absorb } => {
                    commands
                        .entity(active.caster)
                        .insert(ActiveShield { remaining: *absorb });
                }
                AbilityEffect::Buff {
                    stat,
                    multiplier,
                    duration_ticks,
                    target,
                } => {
                    apply_buff(
                        &mut commands,
                        resolve_caster_target(target, active),
                        stat,
                        *multiplier,
                        *duration_ticks,
                        tick,
                    );
                }
                _ => {
                    warn!("Unhandled OnTick effect: {:?}", tick_effect.effect);
                }
            }
        }
    }
}

fn spawn_melee_hitbox(
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

fn spawn_aoe_hitbox(
    commands: &mut Commands,
    ability_entity: Entity,
    active: &ActiveAbility,
    on_hit_effects: Option<&OnHitEffects>,
    caster_query: &Query<(&mut Position, &Rotation, &MapInstanceId)>,
    radius: f32,
    spawn_tick: Tick,
    duration_ticks: u16,
) {
    info!("Spawning AoE hitbox with {duration_ticks:?} lifetime");
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

/// Apply WhileActive effects each tick (e.g. SetVelocity for dashes).
/// Only fires during Active phase.
pub fn apply_while_active_effects(
    query: Query<(&WhileActiveEffects, &ActiveAbility)>,
    mut caster_query: Query<(&Rotation, &mut LinearVelocity)>,
) {
    for (effects, active) in &query {
        if active.phase != AbilityPhase::Active {
            continue;
        }
        for effect in &effects.0 {
            match effect {
                AbilityEffect::SetVelocity { speed, target } => {
                    let target_entity = resolve_caster_target(&target, active);
                    if let Ok((rotation, mut velocity)) = caster_query.get_mut(target_entity) {
                        let direction = facing_direction(rotation);
                        velocity.x = direction.x * speed;
                        velocity.z = direction.z * speed;
                    }
                }
                _ => {
                    warn!("Unhandled WhileActive effect: {:?}", effect);
                }
            }
        }
    }
}

/// Process OnEnd effects — fires on the first Recovery tick (Active->Recovery transition).
/// Does NOT remove OnEndEffects; it's a persistent archetype component.
pub fn apply_on_end_effects(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    ability_assets: Res<Assets<AbilityAsset>>,
    registry: Res<AppTypeRegistry>,
    timeline: Res<LocalTimeline>,
    server_query: Query<&ControlledBy>,
    player_id_query: Query<&PlayerId>,
    query: Query<(Entity, &OnEndEffects, &ActiveAbility)>,
    mut caster_query: Query<(&mut Position, &Rotation, &mut LinearVelocity)>,
) {
    let tick = timeline.tick();
    for (_entity, effects, active) in &query {
        if active.phase != AbilityPhase::Recovery || active.phase_start_tick != tick {
            continue;
        }
        for effect in &effects.0 {
            match effect {
                AbilityEffect::SetVelocity { speed, target } => {
                    let target_entity = resolve_caster_target(target, active);
                    if let Ok((_, rotation, mut velocity)) = caster_query.get_mut(target_entity) {
                        let direction = facing_direction(rotation);
                        velocity.x = direction.x * speed;
                        velocity.z = direction.z * speed;
                    }
                }
                AbilityEffect::Ability { id, target } => {
                    let target_entity = resolve_caster_target(target, active);
                    spawn_sub_ability(
                        &mut commands,
                        ability_defs.as_ref(),
                        ability_assets.as_ref(),
                        &registry.0,
                        id,
                        target_entity,
                        active.original_caster,
                        active.ability_slot,
                        active.depth,
                        tick,
                        &server_query,
                        &player_id_query,
                    );
                }
                AbilityEffect::Teleport { distance } => {
                    let target_entity = resolve_caster_target(&EffectTarget::Caster, active);
                    if let Ok((mut position, rotation, _)) = caster_query.get_mut(target_entity) {
                        let direction = facing_direction(rotation);
                        position.0 += direction * *distance;
                    } else {
                        warn!(
                            "Teleport: caster {:?} missing Position/Rotation",
                            active.caster
                        );
                    }
                }
                AbilityEffect::Shield { absorb } => {
                    commands
                        .entity(active.caster)
                        .insert(ActiveShield { remaining: *absorb });
                }
                AbilityEffect::Buff {
                    stat,
                    multiplier,
                    duration_ticks,
                    target,
                } => {
                    apply_buff(
                        &mut commands,
                        resolve_caster_target(target, active),
                        stat,
                        *multiplier,
                        *duration_ticks,
                        tick,
                    );
                }
                _ => {
                    warn!("Unhandled OnEnd effect: {:?}", effect);
                }
            }
        }
    }
}

/// Process OnInput effects -- checks caster's ActionState for just_pressed inputs
/// and applies matched effects (typically spawning sub-abilities for combo chaining).
/// Only fires during Active phase.
pub fn apply_on_input_effects(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    ability_assets: Res<Assets<AbilityAsset>>,
    registry: Res<AppTypeRegistry>,
    timeline: Res<LocalTimeline>,
    server_query: Query<&ControlledBy>,
    player_id_query: Query<&PlayerId>,
    query: Query<(Entity, &OnInputEffects, &ActiveAbility)>,
    action_query: Query<&ActionState<PlayerActions>>,
) {
    let tick = timeline.tick();
    for (_entity, effects, active) in &query {
        if active.phase != AbilityPhase::Active {
            continue;
        }
        let Ok(action_state) = action_query.get(active.caster) else {
            continue;
        };
        for input_effect in &effects.0 {
            if !action_state.just_pressed(&input_effect.action) {
                continue;
            }
            match &input_effect.effect {
                AbilityEffect::Ability { id, target } => {
                    let target_entity = resolve_caster_target(target, active);
                    spawn_sub_ability(
                        &mut commands,
                        ability_defs.as_ref(),
                        ability_assets.as_ref(),
                        &registry.0,
                        id,
                        target_entity,
                        active.original_caster,
                        active.ability_slot,
                        active.depth,
                        tick,
                        &server_query,
                        &player_id_query,
                    );
                }
                _ => {
                    warn!("Unhandled OnInput effect: {:?}", input_effect.effect);
                }
            }
        }
    }
}

fn apply_teleport(
    caster_query: &mut Query<(&mut Position, &Rotation, &MapInstanceId)>,
    caster: Entity,
    distance: f32,
) {
    if let Ok((mut position, rotation, _)) = caster_query.get_mut(caster) {
        let direction = facing_direction(rotation);
        position.0 += direction * distance;
    } else {
        warn!("Teleport: caster {:?} missing Position/Rotation", caster);
    }
}

fn apply_buff(
    commands: &mut Commands,
    target_entity: Entity,
    stat: &str,
    multiplier: f32,
    duration_ticks: u16,
    tick: Tick,
) {
    let expires_tick = tick + duration_ticks as i16;
    commands
        .entity(target_entity)
        .insert(ActiveBuffs(vec![ActiveBuff {
            stat: stat.to_string(),
            multiplier,
            expires_tick,
        }]));
}

/// Remove expired buffs each tick. Removes the ActiveBuffs component when empty.
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

/// Safety net: remove all effect/archetype markers when `ActiveAbility` is removed.
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

/// Spawn a `AbilityProjectileSpawn` entity from `ProjectileSpawnEffect` markers.
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

/// Spawn child bullet entities from `AbilityProjectileSpawn` parents.
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
        info!("Spawning ability bullet from {:?}", spawn_info.ability_id);
        let mut bullet_cmd = commands.spawn((
            Position(spawn_info.position),
            Rotation::default(),
            LinearVelocity(spawn_info.direction * spawn_info.speed),
            RigidBody::Kinematic,
            Collider::sphere(BULLET_COLLIDER_RADIUS),
            Sensor,
            CollisionEventsEnabled,
            CollidingEntities::default(),
            crate::hit_detection::projectile_collision_layers(),
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

/// When a child bullet's `AbilityBulletOf` is removed, despawn the parent spawn entity.
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

/// Despawn AoE hitboxes whose duration has expired.
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

/// Despawn bullets whose lifetime has expired.
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
