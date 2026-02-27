use avian3d::prelude::*;
use bevy::color::palettes::css;
use bevy::prelude::*;
use bevy_voxel_world::prelude::ChunkRenderTarget;
use leafwing_input_manager::prelude::*;
use lightyear::connection::client::Connected;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::*;
use protocol::*;

pub struct ServerGameplayPlugin;

impl Plugin for ServerGameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(handle_connected);
        app.add_systems(Startup, (spawn_dummy_target, spawn_respawn_points));
        app.add_systems(FixedUpdate, handle_character_movement);
        app.add_systems(
            FixedUpdate,
            (
                check_death_and_respawn.after(hit_detection::process_projectile_hits),
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

fn spawn_dummy_target(mut commands: Commands) {
    commands.spawn((
        Name::new("DummyTarget"),
        Position(Vec3::new(3.0, 30.0, 0.0)),
        Rotation::default(),
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
        CharacterPhysicsBundle::default(),
        ColorComponent(css::GRAY.into()),
        CharacterMarker,
        Health::new(100.0),
        DummyTarget,
        ChunkRenderTarget::<MapWorld>::default(),
    ));
}

fn handle_character_movement(
    time: Res<Time>,
    spatial_query: SpatialQuery,
    mut query: Query<
        (
            Entity,
            &ActionState<PlayerActions>,
            &ComputedMass,
            &Position,
            Forces,
        ),
        With<CharacterMarker>,
    >,
) {
    for (entity, action_state, mass, position, mut forces) in &mut query {
        apply_movement(
            entity,
            mass,
            time.delta_secs(),
            &spatial_query,
            action_state,
            position,
            &mut forces,
        );
    }
}

fn spawn_respawn_points(mut commands: Commands) {
    commands.spawn((RespawnPoint, Position(Vec3::new(0.0, 30.0, 0.0))));
}

fn check_death_and_respawn(
    mut commands: Commands,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut dead_query: Query<
        (Entity, &mut Health, &mut Position, &mut LinearVelocity),
        With<CharacterMarker>,
    >,
    respawn_query: Query<&Position, (With<RespawnPoint>, Without<CharacterMarker>)>,
) {
    let tick = timeline.tick();
    for (entity, mut health, mut position, mut velocity) in &mut dead_query {
        if !health.is_dead() {
            continue;
        }
        let respawn_pos = nearest_respawn_pos(&position, &respawn_query);
        info!("Entity {:?} died, respawning at {:?}", entity, respawn_pos);
        position.0 = respawn_pos;
        velocity.0 = Vec3::ZERO;
        health.restore_full();
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
        .unwrap_or(Vec3::new(0.0, 30.0, 0.0))
}

fn expire_invulnerability(
    mut commands: Commands,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
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

    let angle: f32 = num_characters as f32 * 5.0;
    let x = 2.0 * angle.cos();
    let z = 2.0 * angle.sin();

    commands.spawn((
        Name::new("Character"),
        PlayerId(peer_id),
        Position(Vec3::new(x, 30.0, z)),
        Rotation::default(),
        ActionState::<PlayerActions>::default(),
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
        ControlledBy {
            owner: client_entity,
            lifetime: Default::default(),
        },
        CharacterPhysicsBundle::default(),
        ColorComponent(color.into()),
        CharacterMarker,
        Health::new(100.0),
        ChunkRenderTarget::<MapWorld>::default(),
        AbilityCooldowns::default(),
    ));
}
