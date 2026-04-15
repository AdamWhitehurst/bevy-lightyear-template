use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use lightyear::prelude::client::*;
use protocol::transition::ClientTransitionState;
use ui::*;

fn ui_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.init_resource::<ClientTransitionState>();
    app.add_plugins(UiPlugin);
    app.world_mut()
        .spawn((Name::new("Test Client"), Client::default()));
    app
}

#[test]
fn map_transition_state_defaults_to_playing_when_ingame() {
    let mut app = ui_test_app();

    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::InGame);
    app.update();

    let state = app.world().resource::<State<MapTransitionState>>();
    assert_eq!(
        *state.get(),
        MapTransitionState::Playing,
        "MapTransitionState should default to Playing when entering InGame"
    );
}

#[test]
fn transitioning_state_spawns_loading_screen() {
    let mut app = ui_test_app();

    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::InGame);
    app.update();

    app.world_mut()
        .resource_mut::<NextState<MapTransitionState>>()
        .set(MapTransitionState::Transitioning);
    app.update();

    let mut text_query = app.world_mut().query::<&Text>();
    let has_loading_text = text_query.iter(app.world()).any(|t| t.0 == "Loading...");
    assert!(
        has_loading_text,
        "Loading screen should show 'Loading...' text during Transitioning"
    );
}

#[test]
fn returning_to_playing_despawns_loading_screen() {
    let mut app = ui_test_app();

    // InGame -> Transitioning
    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::InGame);
    app.update();

    app.world_mut()
        .resource_mut::<NextState<MapTransitionState>>()
        .set(MapTransitionState::Transitioning);
    app.update();

    let mut text_query = app.world_mut().query::<&Text>();
    assert!(
        text_query.iter(app.world()).any(|t| t.0 == "Loading..."),
        "Loading screen should exist during Transitioning"
    );

    // Back to Playing
    app.world_mut()
        .resource_mut::<NextState<MapTransitionState>>()
        .set(MapTransitionState::Playing);
    app.update();

    let mut text_query = app.world_mut().query::<&Text>();
    assert!(
        !text_query.iter(app.world()).any(|t| t.0 == "Loading..."),
        "Loading screen should be despawned after returning to Playing"
    );
}

#[test]
fn leaving_ingame_resets_transition_state_on_reentry() {
    let mut app = ui_test_app();

    // InGame -> Transitioning
    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::InGame);
    app.update();

    app.world_mut()
        .resource_mut::<NextState<MapTransitionState>>()
        .set(MapTransitionState::Transitioning);
    app.update();

    // Leave InGame
    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::MainMenu);
    app.update();

    // Re-enter InGame
    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::InGame);
    app.update();

    let state = app.world().resource::<State<MapTransitionState>>();
    assert_eq!(
        *state.get(),
        MapTransitionState::Playing,
        "Re-entering InGame should reset MapTransitionState to Playing"
    );
}
