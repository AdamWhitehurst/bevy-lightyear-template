use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use lightyear::prelude::client::*;
use lightyear::prelude::Predicted;
use protocol::*;
use std::time::Duration;
use ui::*;

#[test]
fn test_ui_plugin_initializes_state() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);

    app.update();

    // Verify state is initialized
    let state = app.world().resource::<State<ClientState>>();
    assert_eq!(*state.get(), ClientState::MainMenu);
}

#[test]
fn test_main_menu_spawns_buttons() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);

    app.update();

    // Verify Connect button exists
    let mut connect_query = app
        .world_mut()
        .query_filtered::<Entity, With<ConnectButton>>();
    assert_eq!(
        connect_query.iter(app.world()).count(),
        1,
        "Should have one Connect button"
    );

    // Verify Quit button exists
    let mut quit_query = app.world_mut().query_filtered::<Entity, With<QuitButton>>();
    assert_eq!(
        quit_query.iter(app.world()).count(),
        1,
        "Should have one Quit button"
    );
}

#[test]
fn test_connect_button_triggers_state_transition() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);

    // Setup dummy client entity (needed for Connecting state)
    app.world_mut()
        .spawn((Name::new("Test Client"), Client::default()));

    app.update();

    // Get connect button
    let button = {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<ConnectButton>>();
        query
            .single(app.world())
            .expect("Connect button should exist")
    };

    // Simulate button press
    app.world_mut()
        .entity_mut(button)
        .insert(Interaction::Pressed);
    app.update();
    app.update(); // Second update for state transition

    // Verify state transitioned to Connecting
    let state = app.world().resource::<State<ClientState>>();
    assert_eq!(*state.get(), ClientState::Connecting);
}

#[test]
fn test_ingame_state_spawns_hud() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);

    // Setup dummy client entity (needed for button interactions)
    app.world_mut()
        .spawn((Name::new("Test Client"), Client::default()));

    // Transition to InGame state
    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::InGame);
    app.update();

    // Verify Main Menu button exists
    let mut main_menu_query = app
        .world_mut()
        .query_filtered::<Entity, With<MainMenuButton>>();
    assert_eq!(
        main_menu_query.iter(app.world()).count(),
        1,
        "Should have one Main Menu button"
    );

    // Verify Quit button exists
    let mut quit_query = app.world_mut().query_filtered::<Entity, With<QuitButton>>();
    assert_eq!(
        quit_query.iter(app.world()).count(),
        1,
        "Should have one Quit button"
    );
}

#[test]
fn test_disconnection_returns_to_main_menu() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(ClientPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
    });
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(UiPlugin);

    // Setup client entity
    app.world_mut()
        .spawn((Name::new("Client"), Client::default()));

    // Set to InGame state
    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::InGame);
    app.update();

    // Verify in InGame state
    let state = app.world().resource::<State<ClientState>>();
    assert_eq!(*state.get(), ClientState::InGame);

    // Trigger disconnection
    let client_entity = {
        let mut query = app.world_mut().query_filtered::<Entity, With<Client>>();
        query.single(app.world()).unwrap()
    };
    app.world_mut()
        .entity_mut(client_entity)
        .insert(Disconnected::default());
    app.update();

    // Verify returned to MainMenu
    let state = app.world().resource::<State<ClientState>>();
    assert_eq!(*state.get(), ClientState::MainMenu);
}

#[test]
fn test_connecting_state_spawns_cancel_button() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);

    // Setup dummy client entity (needed for Connecting state)
    app.world_mut()
        .spawn((Name::new("Test Client"), Client::default()));

    app.update();

    // Transition to Connecting state
    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::Connecting);
    app.update();

    // Verify Cancel button exists
    let mut cancel_query = app
        .world_mut()
        .query_filtered::<Entity, With<CancelButton>>();
    assert_eq!(
        cancel_query.iter(app.world()).count(),
        1,
        "Should have one Cancel button"
    );
}

#[test]
fn ingame_hud_spawns_map_switch_button() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);

    app.world_mut()
        .spawn((Name::new("Test Client"), Client::default()));

    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::InGame);
    app.update();

    let mut query = app
        .world_mut()
        .query_filtered::<Entity, With<MapSwitchButton>>();
    assert_eq!(
        query.iter(app.world()).count(),
        1,
        "Should have one MapSwitchButton"
    );
}

#[test]
fn map_switch_button_label_shows_homebase_when_on_overworld() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);

    app.world_mut()
        .spawn((Name::new("Test Client"), Client::default()));

    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::InGame);
    app.update();

    app.world_mut()
        .spawn((CharacterMarker, Predicted, MapInstanceId::Overworld));
    app.update();

    let button_entity = app
        .world_mut()
        .query_filtered::<Entity, With<MapSwitchButton>>()
        .single(app.world())
        .expect("MapSwitchButton should exist");
    let children = app.world().get::<Children>(button_entity).unwrap();
    let text = app.world().get::<Text>(children[0]).unwrap();
    assert_eq!(
        text.0, "Homebase",
        "Button should say 'Homebase' when player is on Overworld"
    );
}

#[test]
fn map_switch_button_label_shows_overworld_when_on_homebase() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);

    app.world_mut()
        .spawn((Name::new("Test Client"), Client::default()));

    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::InGame);
    app.update();

    app.world_mut().spawn((
        CharacterMarker,
        Predicted,
        MapInstanceId::Homebase { owner: 42 },
    ));
    app.update();

    let button_entity = app
        .world_mut()
        .query_filtered::<Entity, With<MapSwitchButton>>()
        .single(app.world())
        .expect("MapSwitchButton should exist");
    let children = app.world().get::<Children>(button_entity).unwrap();
    let text = app.world().get::<Text>(children[0]).unwrap();
    assert_eq!(
        text.0, "Overworld",
        "Button should say 'Overworld' when player is on Homebase"
    );
}

#[test]
fn test_state_cleanup() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);

    // Setup dummy client entity (needed for Connecting state)
    app.world_mut()
        .spawn((Name::new("Test Client"), Client::default()));

    app.update();

    // Verify main menu UI exists
    let mut main_menu_ui = app
        .world_mut()
        .query_filtered::<Entity, With<ConnectButton>>();
    assert_eq!(main_menu_ui.iter(app.world()).count(), 1);

    // Transition to Connecting state
    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::Connecting);
    app.update();

    // Verify main menu UI is despawned
    let mut main_menu_ui = app
        .world_mut()
        .query_filtered::<Entity, With<ConnectButton>>();
    assert_eq!(
        main_menu_ui.iter(app.world()).count(),
        0,
        "Main menu UI should be despawned"
    );

    // Verify connecting UI exists
    let mut connecting_ui = app
        .world_mut()
        .query_filtered::<Entity, With<CancelButton>>();
    assert_eq!(
        connecting_ui.iter(app.world()).count(),
        1,
        "Connecting UI should exist"
    );
}
