#![cfg(target_family = "wasm")]

use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use lightyear::webtransport::client::WebTransportClientIo;
use protocol::*;
use wasm_bindgen_test::*;
use web::network::WebClientPlugin;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn test_web_client_plugin_spawns_entity() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(WebClientPlugin::default());

    // Run startup systems
    app.update();

    // Verify client entity was spawned
    let mut query = app.world_mut().query::<(
        &Client,
        &NetcodeClient,
        &WebTransportClientIo,
    )>();

    assert!(query.get_single(app.world()).is_ok(), "Web client entity should exist");
}

#[wasm_bindgen_test]
fn test_web_client_plugin_connected_observer() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(WebClientPlugin::default());

    // Run startup to spawn client entity
    app.update();

    // Get the client entity
    let client_entity = app
        .world_mut()
        .query_filtered::<Entity, With<Client>>()
        .single(app.world());

    // Manually trigger Connected event by inserting component
    app.world_mut().entity_mut(client_entity).insert(Connected);

    // Run update to trigger observers
    app.update();

    // Verify observer ran without panicking
    let has_connected = app
        .world()
        .entity(client_entity)
        .contains::<Connected>();
    assert!(has_connected, "Observer should process Connected component");
}

#[wasm_bindgen_test]
fn test_web_client_plugin_disconnected_observer() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(WebClientPlugin::default());

    // Run startup to spawn client entity
    app.update();

    // Get the client entity
    let client_entity = app
        .world_mut()
        .query_filtered::<Entity, With<Client>>()
        .single(app.world());

    // Manually trigger Disconnected event by inserting component
    app.world_mut().entity_mut(client_entity).insert(Disconnected::default());

    // Run update to trigger observers
    app.update();

    // Verify observer ran without panicking
    let has_disconnected = app
        .world()
        .entity(client_entity)
        .contains::<Disconnected>();
    assert!(has_disconnected, "Observer should process Disconnected component");
}
