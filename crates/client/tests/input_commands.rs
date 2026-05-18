use bevy::input::InputPlugin;
use bevy::prelude::*;
use client::input::ownership::{
    apply_egui_ownership_state, ClientInputOwnershipSnapshot, KeyboardInputOwner, PointerInputOwner,
};
use client::input::raw::{raw_client_input_map, RawClientActions};
use client::input::ClientInputCommandPlugin;
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
