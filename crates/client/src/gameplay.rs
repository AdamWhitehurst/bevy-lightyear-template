use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;
use lightyear::prelude::client::input::InputSystems;
use lightyear::prelude::{Controlled, Interpolated, Predicted, Replicated};
use protocol::*;
use render::CameraOrbitState;

use crate::world_object::{
    init_default_vox_model_material, on_visual_kind_changed, on_world_object_replicated,
};

pub struct ClientGameplayPlugin;

impl Plugin for ClientGameplayPlugin {
    fn build(&self, app: &mut App) {
        let ready = in_state(AppState::Ready);
        app.add_systems(Startup, init_default_vox_model_material);
        app.add_systems(Update, handle_new_character);
        // detect_grounded must run before handle_character_movement and
        // ability_activation so the IsGrounded gate sees fresh state.
        app.add_systems(
            FixedUpdate,
            (protocol::detect_grounded, handle_character_movement)
                .chain()
                .before(protocol::ability::ability_activation),
        );
        app.add_systems(
            FixedPreUpdate,
            sync_camera_yaw_to_input.before(InputSystems::BufferClientInputs),
        );
        app.add_systems(
            Update,
            (on_world_object_replicated, on_visual_kind_changed).run_if(ready),
        );

        app.add_observer(on_respawn_timer_added);
        app.add_observer(on_respawn_timer_removed);
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
    registry: Res<MapRegistry>,
    map_ids: Query<&MapInstanceId>,
) {
    for (entity, is_controlled) in &confirmed_query {
        if is_controlled {
            trace!("Adding InputMap to controlled and predicted entity {entity:?}");
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
            trace!("Remote character predicted for us: {entity:?}");
        }
    }

    for entity in &character_query {
        if let Ok(mid) = map_ids.get(entity) {
            if !registry.0.contains_key(mid) {
                trace!("Despawning stale character {entity:?} from map {mid:?}");
                commands.entity(entity).despawn();
                continue;
            }
        }
        trace!(?entity, "Adding physics to predicted character");
        commands
            .entity(entity)
            .insert(CharacterPhysicsBundle::default());
    }
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
        (
            With<Predicted>,
            With<CharacterMarker>,
            Without<RespawnTimer>,
        ),
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

/// Hides entity and descendants when a respawn timer is added.
fn on_respawn_timer_added(
    trigger: On<Add, RespawnTimer>,
    mut commands: Commands,
    children_query: Query<&Children>,
) {
    let entity = trigger.entity;
    commands
        .entity(entity)
        .insert((Visibility::Hidden, RigidBodyDisabled, ColliderDisabled));
    set_descendants_visibility(&mut commands, entity, &children_query, Visibility::Hidden);
}

/// Restores entity and descendants when respawn timer is removed.
fn on_respawn_timer_removed(
    trigger: On<Remove, RespawnTimer>,
    mut commands: Commands,
    children_query: Query<&Children>,
) {
    let entity = trigger.entity;
    commands
        .entity(entity)
        .remove::<(RigidBodyDisabled, ColliderDisabled)>()
        .insert(Visibility::Inherited);
    set_descendants_visibility(
        &mut commands,
        entity,
        &children_query,
        Visibility::Inherited,
    );
}

/// Recursively sets visibility on all descendants of an entity.
fn set_descendants_visibility(
    commands: &mut Commands,
    entity: Entity,
    children_query: &Query<&Children>,
    visibility: Visibility,
) {
    let Ok(children) = children_query.get(entity) else {
        return;
    };
    for &child in children {
        commands.entity(child).insert(visibility);
        set_descendants_visibility(commands, child, children_query, visibility);
    }
}

/// Writes the camera's target yaw angle into the player's ActionState for replication.
fn sync_camera_yaw_to_input(
    camera_query: Query<&CameraOrbitState>,
    mut player_query: Query<&mut ActionState<PlayerActions>, With<Predicted>>,
) {
    let Ok(orbit) = camera_query.single() else {
        return;
    };

    for mut action_state in &mut player_query {
        action_state.set_value(&PlayerActions::CameraYaw, orbit.target_angle);
    }
}
