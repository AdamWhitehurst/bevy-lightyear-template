use bevy::input::InputPlugin;
use bevy::prelude::*;
use client::input::editor::{TerrainCommandIntent, WorldObjectCommandIntent};
use client::input::gestures::ClientPointerGestureState;
use client::input::ownership::{
    apply_editing_mode_pointer_ownership, apply_egui_ownership_state, ClientInputOwnershipSnapshot,
    KeyboardInputOwner, PointerInputOwner,
};
use client::input::raw::{raw_client_input_map, RawClientActions};
use client::input::ClientInputCommandPlugin;
use dev::{panels::spawn::SpawnPanelUi, EditingMode};
use leafwing_input_manager::prelude::*;
use lightyear::prelude::client::input::InputSystems;
use lightyear::prelude::Controlled;
use protocol::NetworkedPlayerActions;

#[derive(Resource, Default)]
struct BufferedObserved(bool);

fn observe_buffered_input(
    query: Query<&ActionState<NetworkedPlayerActions>, With<Controlled>>,
    mut observed: ResMut<BufferedObserved>,
) {
    let action_state = query.single().expect("controlled input entity exists");
    observed.0 = action_state.just_pressed(&NetworkedPlayerActions::Ability1);
}

fn command_test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, InputPlugin));
    app.add_plugins(ClientInputCommandPlugin);
    app.init_resource::<BufferedObserved>();
    app.add_systems(
        FixedPreUpdate,
        observe_buffered_input.in_set(InputSystems::BufferClientInputs),
    );
    app
}

fn spawn_controlled_input(app: &mut App) -> Entity {
    app.world_mut()
        .spawn((
            Controlled,
            ActionState::<RawClientActions>::default(),
            raw_client_input_map(),
            ActionState::<NetworkedPlayerActions>::default(),
            InputMap::<NetworkedPlayerActions>::default(),
        ))
        .id()
}

fn press_raw_ability_1(app: &mut App, entity: Entity) {
    app.world_mut()
        .get_mut::<ActionState<RawClientActions>>(entity)
        .expect("raw action state exists")
        .press(&RawClientActions::Ability1);
}

fn run_fixed_pre_update(app: &mut App) {
    app.world_mut().run_schedule(FixedPreUpdate);
}

#[test]
fn client_input_plugin_initializes_command_resources() {
    let app = command_test_app();
    assert!(app
        .world()
        .contains_resource::<ClientInputOwnershipSnapshot>());
    assert!(app.world().contains_resource::<ClientPointerGestureState>());
    assert!(app
        .world()
        .contains_resource::<Messages<TerrainCommandIntent>>());
    assert!(app
        .world()
        .contains_resource::<Messages<WorldObjectCommandIntent>>());
}

#[test]
fn raw_ability_hotkey_populates_raw_client_action() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, InputPlugin));
    app.add_plugins(InputManagerPlugin::<RawClientActions>::default());
    let entity = app
        .world_mut()
        .spawn((
            ActionState::<RawClientActions>::default(),
            raw_client_input_map(),
        ))
        .id();

    KeyCode::Digit1.press(app.world_mut());
    app.update();

    let action_state = app
        .world()
        .get::<ActionState<RawClientActions>>(entity)
        .expect("raw action state exists");
    assert!(action_state.pressed(&RawClientActions::Ability1));
}

#[test]
fn ability_hotkey_writes_networked_action_when_keyboard_owned_by_gameplay() {
    let mut app = command_test_app();
    let entity = spawn_controlled_input(&mut app);
    press_raw_ability_1(&mut app, entity);

    run_fixed_pre_update(&mut app);

    let action_state = app
        .world()
        .get::<ActionState<NetworkedPlayerActions>>(entity)
        .expect("networked action state exists");
    assert!(action_state.just_pressed(&NetworkedPlayerActions::Ability1));
}

#[test]
fn ability_does_not_fire_when_ui_owns_keyboard() {
    let mut app = command_test_app();
    let entity = spawn_controlled_input(&mut app);
    app.world_mut()
        .resource_mut::<ClientInputOwnershipSnapshot>()
        .keyboard = KeyboardInputOwner::Ui;
    press_raw_ability_1(&mut app, entity);

    run_fixed_pre_update(&mut app);

    let action_state = app
        .world()
        .get::<ActionState<NetworkedPlayerActions>>(entity)
        .expect("networked action state exists");
    assert!(!action_state.pressed(&NetworkedPlayerActions::Ability1));
}

#[test]
fn ability_does_not_fire_when_text_owns_keyboard() {
    let mut app = command_test_app();
    let entity = spawn_controlled_input(&mut app);
    app.world_mut()
        .resource_mut::<ClientInputOwnershipSnapshot>()
        .keyboard = KeyboardInputOwner::Text;
    press_raw_ability_1(&mut app, entity);

    run_fixed_pre_update(&mut app);

    let action_state = app
        .world()
        .get::<ActionState<NetworkedPlayerActions>>(entity)
        .expect("networked action state exists");
    assert!(!action_state.pressed(&NetworkedPlayerActions::Ability1));
}

#[test]
fn egui_keyboard_focus_captures_text_ownership() {
    let mut ownership = ClientInputOwnershipSnapshot::default();

    apply_egui_ownership_state(&mut ownership, true, false);

    assert_eq!(ownership.keyboard, KeyboardInputOwner::Text);
    assert_eq!(ownership.pointer, PointerInputOwner::World);
}

#[test]
fn egui_pointer_focus_captures_ui_pointer_ownership() {
    let mut ownership = ClientInputOwnershipSnapshot::default();

    apply_egui_ownership_state(&mut ownership, false, true);

    assert_eq!(ownership.keyboard, KeyboardInputOwner::Gameplay);
    assert_eq!(ownership.pointer, PointerInputOwner::Ui);
}

#[test]
fn ability_input_filter_runs_before_lightyear_buffers_inputs() {
    let mut app = command_test_app();
    let entity = spawn_controlled_input(&mut app);
    press_raw_ability_1(&mut app, entity);

    run_fixed_pre_update(&mut app);

    assert!(app.world().resource::<BufferedObserved>().0);
}

fn pointer_test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, InputPlugin));
    app.add_plugins(ClientInputCommandPlugin);
    app
}

fn run_pointer_frame(app: &mut App) {
    app.world_mut().run_schedule(FixedPreUpdate);
}

fn press_raw_place(app: &mut App, entity: Entity) {
    app.world_mut()
        .get_mut::<ActionState<RawClientActions>>(entity)
        .expect("raw action state exists")
        .press(&RawClientActions::PlaceVoxel);
}

fn release_raw_place(app: &mut App, entity: Entity) {
    app.world_mut()
        .get_mut::<ActionState<RawClientActions>>(entity)
        .expect("raw action state exists")
        .release(&RawClientActions::PlaceVoxel);
}

fn press_raw_delete(app: &mut App, entity: Entity) {
    app.world_mut()
        .get_mut::<ActionState<RawClientActions>>(entity)
        .expect("raw action state exists")
        .press(&RawClientActions::Delete);
}

fn current_world_object_intents(app: &App) -> Vec<WorldObjectCommandIntent> {
    app.world()
        .resource::<Messages<WorldObjectCommandIntent>>()
        .iter_current_update_messages()
        .cloned()
        .collect()
}

fn run_world_object_command_frame(app: &mut App) {
    app.world_mut().run_schedule(FixedPreUpdate);
}

#[test]
fn pointer_press_over_ui_latches_ui_owner_until_release() {
    let mut app = pointer_test_app();
    let entity = spawn_controlled_input(&mut app);
    app.world_mut()
        .resource_mut::<ClientInputOwnershipSnapshot>()
        .pointer = PointerInputOwner::Ui;
    press_raw_place(&mut app, entity);

    run_pointer_frame(&mut app);

    assert_eq!(
        app.world().resource::<ClientPointerGestureState>().owner,
        Some(PointerInputOwner::Ui)
    );

    app.world_mut()
        .resource_mut::<ClientInputOwnershipSnapshot>()
        .pointer = PointerInputOwner::TerrainBrush;
    run_pointer_frame(&mut app);

    assert_eq!(
        app.world().resource::<ClientPointerGestureState>().owner,
        Some(PointerInputOwner::Ui),
        "active drags keep their initial pointer owner"
    );

    release_raw_place(&mut app, entity);
    run_pointer_frame(&mut app);

    assert_eq!(
        app.world().resource::<ClientPointerGestureState>().owner,
        None
    );
}

#[test]
fn pointer_press_over_terrain_latches_terrain_owner_until_release() {
    let mut app = pointer_test_app();
    let entity = spawn_controlled_input(&mut app);
    app.world_mut()
        .resource_mut::<ClientInputOwnershipSnapshot>()
        .pointer = PointerInputOwner::TerrainBrush;
    press_raw_place(&mut app, entity);

    run_pointer_frame(&mut app);

    assert_eq!(
        app.world().resource::<ClientPointerGestureState>().owner,
        Some(PointerInputOwner::TerrainBrush)
    );

    release_raw_place(&mut app, entity);
    run_pointer_frame(&mut app);

    assert_eq!(
        app.world().resource::<ClientPointerGestureState>().owner,
        None
    );
}

#[test]
fn terrain_does_not_edit_when_ui_owns_pointer() {
    assert!(!PointerInputOwner::Ui.allows_terrain());
}

#[test]
fn terrain_does_not_edit_when_world_object_owns_pointer() {
    assert!(!PointerInputOwner::WorldObject.allows_terrain());
}

#[test]
fn terrain_mode_primary_action_emits_only_terrain_intent() {
    let mut ownership = ClientInputOwnershipSnapshot::default();

    apply_editing_mode_pointer_ownership(&mut ownership, EditingMode::Terrain);

    assert_eq!(ownership.pointer, PointerInputOwner::TerrainBrush);
    assert!(ownership.pointer.allows_terrain());
    assert!(!ownership.pointer.allows_world_object());
}

#[test]
fn place_definition_mode_primary_action_emits_only_world_object_place_intent() {
    let mut app = pointer_test_app();
    app.init_resource::<SpawnPanelUi>();
    *app.world_mut().resource_mut::<EditingMode>() = EditingMode::PlaceDefinition;
    app.world_mut()
        .resource_mut::<SpawnPanelUi>()
        .placement
        .armed = true;
    let entity = spawn_controlled_input(&mut app);
    press_raw_place(&mut app, entity);

    run_world_object_command_frame(&mut app);

    assert_eq!(
        current_world_object_intents(&app),
        vec![WorldObjectCommandIntent::Place]
    );
    assert!(PointerInputOwner::WorldObject.allows_world_object());
    assert!(!PointerInputOwner::WorldObject.allows_terrain());
}

#[test]
fn world_object_place_does_not_fire_when_ui_owns_pointer() {
    let mut app = pointer_test_app();
    app.init_resource::<SpawnPanelUi>();
    *app.world_mut().resource_mut::<EditingMode>() = EditingMode::PlaceDefinition;
    app.world_mut()
        .resource_mut::<SpawnPanelUi>()
        .placement
        .armed = true;
    app.world_mut()
        .resource_mut::<ClientPointerGestureState>()
        .owner = Some(PointerInputOwner::Ui);
    let entity = spawn_controlled_input(&mut app);
    press_raw_place(&mut app, entity);

    run_world_object_command_frame(&mut app);

    assert!(current_world_object_intents(&app).is_empty());
}

#[test]
fn world_object_place_does_not_fire_when_terrain_owns_pointer() {
    let mut app = pointer_test_app();
    app.init_resource::<SpawnPanelUi>();
    *app.world_mut().resource_mut::<EditingMode>() = EditingMode::PlaceDefinition;
    app.world_mut()
        .resource_mut::<SpawnPanelUi>()
        .placement
        .armed = true;
    app.world_mut()
        .resource_mut::<ClientPointerGestureState>()
        .owner = Some(PointerInputOwner::TerrainBrush);
    let entity = spawn_controlled_input(&mut app);
    press_raw_place(&mut app, entity);

    run_world_object_command_frame(&mut app);

    assert!(current_world_object_intents(&app).is_empty());
}

#[test]
fn select_edit_mode_primary_action_emits_pick_or_edit_intent_only() {
    let mut app = pointer_test_app();
    app.init_resource::<SpawnPanelUi>();
    *app.world_mut().resource_mut::<EditingMode>() = EditingMode::SelectEdit;
    app.world_mut()
        .resource_mut::<SpawnPanelUi>()
        .selection
        .cursor_pick_armed = true;
    let entity = spawn_controlled_input(&mut app);
    press_raw_place(&mut app, entity);

    run_world_object_command_frame(&mut app);

    assert_eq!(
        current_world_object_intents(&app),
        vec![WorldObjectCommandIntent::Pick]
    );
}

#[test]
fn world_object_keyboard_delete_does_not_fire_when_ui_or_text_owns_keyboard() {
    for owner in [KeyboardInputOwner::Ui, KeyboardInputOwner::Text] {
        let mut app = pointer_test_app();
        app.init_resource::<SpawnPanelUi>();
        *app.world_mut().resource_mut::<EditingMode>() = EditingMode::SelectEdit;
        app.world_mut()
            .resource_mut::<ClientInputOwnershipSnapshot>()
            .keyboard = owner;
        let entity = spawn_controlled_input(&mut app);
        press_raw_delete(&mut app, entity);

        run_world_object_command_frame(&mut app);

        assert!(current_world_object_intents(&app).is_empty());
    }
}

#[test]
fn panel_delete_and_keyboard_delete_share_delete_intent() {
    let mut panel_app = pointer_test_app();
    panel_app.init_resource::<SpawnPanelUi>();
    *panel_app.world_mut().resource_mut::<EditingMode>() = EditingMode::SelectEdit;
    panel_app
        .world_mut()
        .resource_mut::<SpawnPanelUi>()
        .selection
        .delete_requested = true;

    run_world_object_command_frame(&mut panel_app);

    assert_eq!(
        current_world_object_intents(&panel_app),
        vec![WorldObjectCommandIntent::Delete]
    );

    let mut keyboard_app = pointer_test_app();
    keyboard_app.init_resource::<SpawnPanelUi>();
    *keyboard_app.world_mut().resource_mut::<EditingMode>() = EditingMode::SelectEdit;
    let entity = spawn_controlled_input(&mut keyboard_app);
    press_raw_delete(&mut keyboard_app, entity);

    run_world_object_command_frame(&mut keyboard_app);

    assert_eq!(
        current_world_object_intents(&keyboard_app),
        vec![WorldObjectCommandIntent::Delete]
    );
}
