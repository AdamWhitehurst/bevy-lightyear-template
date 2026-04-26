use ::client::network::ClientNetworkPlugin;
use bevy::prelude::*;
use lightyear::prelude::client as lightyear_client;
use lightyear::prelude::*;
use lightyear_client::*;
use protocol::*;

#[test]
fn test_client_network_plugin_registers_observers() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(ClientNetworkPlugin::default());

    // Run startup to spawn client entity
    app.update();

    // Get the client entity
    let mut query = app.world_mut().query_filtered::<Entity, With<Client>>();
    let client_entity = query.single(app.world()).unwrap();

    // Manually trigger Connected event by inserting component (with required RemoteId)
    app.world_mut()
        .entity_mut(client_entity)
        .insert((Connected, RemoteId(PeerId::Netcode(0))));

    // Run update to trigger observers
    app.update();

    // Verify observer ran without panicking and Connected component persists
    let has_connected = app.world().entity(client_entity).contains::<Connected>();
    assert!(
        has_connected,
        "Observer should process Connected component without removing it"
    );
}

#[test]
fn test_client_network_plugin_disconnected_observer() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(ClientNetworkPlugin::default());

    // Run startup to spawn client entity
    app.update();

    // Get the client entity
    let mut query = app.world_mut().query_filtered::<Entity, With<Client>>();
    let client_entity = query.single(app.world()).unwrap();

    // Manually trigger Disconnected event by inserting component
    app.world_mut()
        .entity_mut(client_entity)
        .insert(Disconnected::default());

    // Run update to trigger observers
    app.update();

    // Verify observer ran without panicking
    let has_disconnected = app.world().entity(client_entity).contains::<Disconnected>();
    assert!(
        has_disconnected,
        "Observer should process Disconnected component"
    );
}
