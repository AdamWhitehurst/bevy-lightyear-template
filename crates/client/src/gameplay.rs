use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;
use lightyear::prelude::{Controlled, Interpolated, Predicted, Replicated};
use protocol::*;

pub struct ClientGameplayPlugin;

impl Plugin for ClientGameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, handle_new_character);
        app.add_systems(FixedUpdate, handle_character_movement);
    }
}

fn handle_new_character(
    mut commands: Commands,
    confirmed_query: Query<(Entity, Has<Controlled>), (Added<Replicated>, With<CharacterMarker>)>,
    character_query: Query<
        Entity,
        (
            Or<(Added<Predicted>, Added<Interpolated>)>,
            With<CharacterMarker>,
        ),
    >,
) {
    for (entity, is_controlled) in &confirmed_query {
        if is_controlled {
            info!("Adding InputMap to controlled and predicted entity {entity:?}");
            commands.entity(entity).insert(
                InputMap::new([(PlayerActions::Jump, KeyCode::Space)])
                    .with(PlayerActions::Jump, GamepadButton::South)
                    .with_dual_axis(PlayerActions::Move, GamepadStick::LEFT)
                    .with_dual_axis(PlayerActions::Move, VirtualDPad::wasd())
                    .with(PlayerActions::PlaceVoxel, MouseButton::Left)
                    .with(PlayerActions::RemoveVoxel, MouseButton::Right)
                    .with(PlayerActions::Ability1, KeyCode::Digit1)
                    .with(PlayerActions::Ability2, KeyCode::Digit2)
                    .with(PlayerActions::Ability3, KeyCode::Digit3)
                    .with(PlayerActions::Ability4, KeyCode::Digit4),
            );
        } else {
            info!("Remote character predicted for us: {entity:?}");
        }
    }

    for entity in &character_query {
        info!(?entity, "Adding physics to predicted character");
        commands
            .entity(entity)
            .insert(CharacterPhysicsBundle::default());
    }
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
        (With<Predicted>, With<CharacterMarker>, Without<ActiveAbility>),
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
