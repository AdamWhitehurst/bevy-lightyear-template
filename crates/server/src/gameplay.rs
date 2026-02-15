use avian3d::prelude::*;
use bevy::color::palettes::css;
use bevy::prelude::*;
use bevy_voxel_world::prelude::ChunkRenderTarget;
use leafwing_input_manager::prelude::*;
use lightyear::connection::client::Connected;
use lightyear::prelude::*;
use protocol::*;

pub struct ServerGameplayPlugin;

impl Plugin for ServerGameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(handle_connected);
        app.add_systems(Startup, spawn_dummy_target);
        app.add_systems(FixedUpdate, handle_character_movement);
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
        (With<CharacterMarker>, Without<ActiveAbility>),
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

fn handle_connected(
    trigger: On<Add, Connected>,
    mut commands: Commands,
    character_query: Query<Entity, (With<CharacterMarker>, Without<DummyTarget>)>,
) {
    let client_entity = trigger.entity;
    info!("Client {client_entity:?} connected. Spawning character entity.");

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
        ChunkRenderTarget::<MapWorld>::default(),
        AbilitySlots([
            Some(AbilityId("punch".into())),
            Some(AbilityId("dash".into())),
            Some(AbilityId("fireball".into())),
            None,
        ]),
        AbilityCooldowns::default(),
    ));
}
