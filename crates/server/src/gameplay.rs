use avian3d::prelude::*;
use bevy::color::palettes::css;
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;
use lightyear::connection::client::Connected;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::*;
use protocol::vox_model::{VoxModelAsset, VoxModelRegistry};
use protocol::world_object::{
    ActiveTransformation, DeathEffect, OnDeathEffects, WorldObjectDefRegistry, WorldObjectId,
};
use protocol::*;

use crate::map::{load_startup_entities, ClientChunkVisibility};
use voxel_map_engine::prelude::ChunkTicket;

/// Default spawn position used for respawning and initial player placement.
pub const DEFAULT_SPAWN_POS: Vec3 = Vec3::new(0.0, 5.0, 0.0);

pub struct ServerGameplayPlugin;

impl Plugin for ServerGameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(handle_connected);
        app.add_systems(
            OnEnter(AppState::Ready),
            (spawn_dummy_target, validate_respawn_points).after(load_startup_entities),
        );
        app.add_systems(FixedUpdate, handle_character_movement);
        app.add_message::<DeathEvent>();
        app.add_systems(
            FixedUpdate,
            (
                on_death_effects
                    .after(hit_detection::process_projectile_hits)
                    .after(hit_detection::process_hitbox_hits)
                    .run_if(
                        resource_exists::<WorldObjectDefRegistry>
                            .and(resource_exists::<VoxModelRegistry>),
                    ),
                start_respawn_timer
                    .after(hit_detection::process_projectile_hits)
                    .after(hit_detection::process_hitbox_hits),
                tick_active_transformations.run_if(
                    resource_exists::<WorldObjectDefRegistry>
                        .and(resource_exists::<VoxModelRegistry>),
                ),
                process_respawn_timers.after(start_respawn_timer),
                expire_invulnerability,
            ),
        );
        app.add_systems(Update, sync_ability_manifest);
    }
}

const ABILITY_MANIFEST_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/abilities.manifest.ron"
);

/// Writes `abilities.manifest.ron` whenever `AbilityDefs` changes, keeping the
/// manifest in sync for WASM web builds.
fn sync_ability_manifest(defs: Option<Res<AbilityDefs>>, mut last_len: Local<usize>) {
    let Some(defs) = defs else { return };
    if !defs.is_changed() && defs.abilities.len() == *last_len {
        return;
    }
    *last_len = defs.abilities.len();

    let mut ids: Vec<&str> = defs.abilities.keys().map(|id| id.0.as_str()).collect();
    ids.sort_unstable();

    match ron::to_string(&ids) {
        Ok(content) => {
            if let Err(e) = std::fs::write(ABILITY_MANIFEST_PATH, content) {
                warn!("Failed to write ability manifest: {e}");
            }
        }
        Err(e) => warn!("Failed to serialize ability manifest: {e}"),
    }
}

fn spawn_dummy_target(mut commands: Commands, registry: Res<MapRegistry>) {
    commands.spawn((
        Name::new("DummyTarget"),
        Position(Vec3::new(10.0, 5.0, 0.0)),
        Rotation::default(),
        Replicate::to_clients(NetworkTarget::All),
        NetworkVisibility,
        PredictionTarget::to_clients(NetworkTarget::All),
        CharacterPhysicsBundle::default(),
        ColorComponent(css::GRAY.into()),
        CharacterMarker,
        CharacterType::Humanoid,
        MapInstanceId::Overworld,
        Health::new(100.0),
        RespawnTimerConfig::default(),
        ChunkTicket::npc(registry.get(&MapInstanceId::Overworld)),
        DummyTarget,
    ));
}

fn handle_character_movement(
    time: Res<Time>,
    spatial_query: SpatialQuery,
    map_ids: Query<&MapInstanceId>,
    mut query: Query<
        (
            Entity,
            &ActionState<PlayerActions>,
            &ComputedMass,
            &Position,
            Forces,
            Option<&MapInstanceId>,
        ),
        (With<CharacterMarker>, Without<RespawnTimer>),
    >,
) {
    for (entity, action_state, mass, position, mut forces, player_map_id) in &mut query {
        apply_movement(
            entity,
            mass,
            time.delta_secs(),
            &spatial_query,
            action_state,
            position,
            &mut forces,
            player_map_id,
            &map_ids,
        );
    }
}

/// Ensures every registered map has at least one respawn point.
/// On first run (no save), spawns a default. On subsequent runs, loaded from disk.
fn validate_respawn_points(
    mut commands: Commands,
    existing: Query<(&RespawnPoint, &MapInstanceId)>,
    map_registry: Res<MapRegistry>,
) {
    for (map_id, _entity) in map_registry.0.iter() {
        let has_respawn = existing.iter().any(|(_, mid)| mid == map_id);
        if !has_respawn {
            trace!("Map {map_id:?} has no respawn points — spawning default");
            commands.spawn((RespawnPoint, Position(DEFAULT_SPAWN_POS), map_id.clone()));
        }
    }
}

/// Starts respawn timers for entities that just died (via DeathEvent).
/// Skips entities with `OnDeathEffects` — those are handled by `on_death_effects`.
fn start_respawn_timer(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    mut events: MessageReader<DeathEvent>,
    query: Query<
        (Option<&RespawnTimerConfig>, Has<OnDeathEffects>),
        (Without<RespawnTimer>, Without<RespawnPoint>),
    >,
) {
    let tick = timeline.tick();
    for event in events.read() {
        let Ok((config, has_death_effects)) = query.get(event.entity) else {
            continue;
        };
        if has_death_effects {
            continue;
        }
        let duration = config
            .map(|c| c.duration_ticks)
            .unwrap_or(DEFAULT_RESPAWN_TICKS);
        commands.entity(event.entity).insert((
            RespawnTimer {
                expires_at: tick + duration as i16,
            },
            RigidBodyDisabled,
            ColliderDisabled,
        ));
    }
}

/// Processes death effects for world objects that just died.
fn on_death_effects(
    mut commands: Commands,
    mut events: MessageReader<DeathEvent>,
    effect_query: Query<(&OnDeathEffects, &WorldObjectId)>,
    defs: Res<WorldObjectDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
) {
    for event in events.read() {
        let Ok((effects, obj_id)) = effect_query.get(event.entity) else {
            continue;
        };
        for effect in &effects.0 {
            match effect {
                DeathEffect::TransformInto {
                    source,
                    revert_after_ticks,
                } => {
                    let source_id = WorldObjectId(source.clone());
                    let Some(source_def) = defs.get(&source_id) else {
                        warn!("Unknown transformation source '{source}'");
                        continue;
                    };
                    let Some(current_def) = defs.get(obj_id) else {
                        warn!("Unknown current def '{}'", obj_id.0);
                        continue;
                    };
                    crate::world_object::apply_transformation(
                        &mut commands,
                        event.entity,
                        current_def,
                        source_def,
                        &type_registry,
                        &vox_registry,
                        &vox_assets,
                        &meshes,
                    );
                    commands.entity(event.entity).insert(ActiveTransformation {
                        source: source.clone(),
                        ticks_remaining: *revert_after_ticks,
                    });
                }
            }
        }
    }
}

/// Decrements active transformation timers. Triggers revert when countdown reaches zero.
fn tick_active_transformations(
    mut commands: Commands,
    mut query: Query<(Entity, &mut ActiveTransformation, &WorldObjectId)>,
    defs: Res<WorldObjectDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
) {
    for (entity, mut transform, obj_id) in &mut query {
        let Some(ref mut remaining) = transform.ticks_remaining else {
            continue;
        };
        *remaining = remaining.saturating_sub(1);
        if *remaining > 0 {
            continue;
        }

        let source_id = WorldObjectId(transform.source.clone());
        let Some(source_def) = defs.get(&source_id) else {
            warn!("Cannot revert: unknown source def '{}'", transform.source);
            continue;
        };
        let Some(original_def) = defs.get(obj_id) else {
            warn!("Cannot revert: unknown original def '{}'", obj_id.0);
            continue;
        };

        crate::world_object::apply_transformation(
            &mut commands,
            entity,
            source_def,
            original_def,
            &type_registry,
            &vox_registry,
            &vox_assets,
            &meshes,
        );
        commands.entity(entity).remove::<ActiveTransformation>();
    }
}

/// Processes expired respawn timers: teleports, heals, grants invulnerability.
fn process_respawn_timers(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    mut query: Query<
        (
            Entity,
            &RespawnTimer,
            &mut Health,
            &mut Position,
            Option<&mut LinearVelocity>,
            Option<&CharacterMarker>,
        ),
        Without<RespawnPoint>,
    >,
    respawn_query: Query<&Position, (With<RespawnPoint>, Without<CharacterMarker>)>,
) {
    let tick = timeline.tick();
    for (entity, timer, mut health, mut position, velocity, character) in &mut query {
        if tick < timer.expires_at {
            continue;
        }
        let respawn_pos = if character.is_some() {
            nearest_respawn_pos(&position, &respawn_query)
        } else {
            position.0
        };
        trace!(
            "Entity {:?} respawn timer expired, respawning at {:?}",
            entity,
            respawn_pos
        );
        position.0 = respawn_pos;
        if let Some(mut velocity) = velocity {
            velocity.0 = Vec3::ZERO;
        }
        health.restore_full();
        commands
            .entity(entity)
            .remove::<(RespawnTimer, RigidBodyDisabled, ColliderDisabled)>();
        commands.entity(entity).insert(Invulnerable {
            expires_at: tick + 128i16,
        });
    }
}

fn nearest_respawn_pos(
    current_pos: &Position,
    respawn_query: &Query<&Position, (With<RespawnPoint>, Without<CharacterMarker>)>,
) -> Vec3 {
    respawn_query
        .iter()
        .min_by(|a, b| {
            a.0.distance_squared(current_pos.0)
                .partial_cmp(&b.0.distance_squared(current_pos.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| p.0)
        .unwrap_or(DEFAULT_SPAWN_POS)
}

fn expire_invulnerability(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    query: Query<(Entity, &Invulnerable)>,
) {
    let tick = timeline.tick();
    for (entity, invuln) in &query {
        if tick >= invuln.expires_at {
            commands.entity(entity).remove::<Invulnerable>();
        }
    }
}

fn handle_connected(
    trigger: On<Add, Connected>,
    mut commands: Commands,
    character_query: Query<Entity, (With<CharacterMarker>, Without<DummyTarget>)>,
    remote_id_query: Query<&RemoteId, With<ClientOf>>,
    registry: Res<MapRegistry>,
    mut room_registry: ResMut<crate::map::RoomRegistry>,
    respawn_query: Query<(&Position, &MapInstanceId), With<RespawnPoint>>,
) {
    let client_entity = trigger.entity;
    let peer_id = remote_id_query
        .get(client_entity)
        .expect("Connected client should have RemoteId")
        .0;
    info!("Client {peer_id} connected. Spawning character entity.");

    let num_characters = character_query.iter().count();

    let available_colors = [
        css::LIMEGREEN,
        css::PINK,
        css::YELLOW,
        css::AQUA,
        css::CRIMSON,
    ];
    let color = available_colors[num_characters % available_colors.len()];

    let spawn_pos = respawn_query
        .iter()
        .find(|(_, mid)| **mid == MapInstanceId::Overworld)
        .map(|(p, _)| p.0)
        .unwrap_or(DEFAULT_SPAWN_POS);

    commands
        .spawn((
            Name::new("Character"),
            PlayerId(peer_id),
            Position(spawn_pos),
            Rotation::default(),
            ActionState::<PlayerActions>::default(),
            Replicate::to_clients(NetworkTarget::All),
            NetworkVisibility,
            PredictionTarget::to_clients(NetworkTarget::All),
            ControlledBy {
                owner: client_entity,
                lifetime: Default::default(),
            },
            CharacterPhysicsBundle::default(),
            ColorComponent(color.into()),
            CharacterMarker,
            CharacterType::Humanoid,
            MapInstanceId::Overworld,
        ))
        .insert((
            Health::new(100.0),
            RespawnTimerConfig::default(),
            AbilityCooldowns::default(),
            ChunkTicket::player(registry.get(&MapInstanceId::Overworld)),
            ClientChunkVisibility::default(),
        ));

    let room = room_registry.get_or_create(&MapInstanceId::Overworld, &mut commands);
    commands.trigger(RoomEvent {
        room,
        target: RoomTarget::AddSender(client_entity),
    });
}
