use ::client::network::{ClientNetworkConfig, ClientNetworkPlugin};
use bevy::prelude::*;
use lightyear::prelude::client as lightyear_client;
use lightyear::prelude::*;
use lightyear_client::*;
use protocol::*;
use std::net::SocketAddr;

#[test]
fn test_client_network_plugin_spawns_entity() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);

    // Add plugin with custom config
    let config = ClientNetworkConfig {
        client_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
        server_addr: SocketAddr::from(([127, 0, 0, 1], 5000)),
        transport: ::client::network::ClientTransport::Udp,
        ..Default::default()
    };
    app.add_plugins(ClientNetworkPlugin {
        config: config.clone(),
    });

    // Run startup systems
    app.update();

    // Verify client entity was spawned with correct components
    let mut query = app
        .world_mut()
        .query::<(&Client, &NetcodeClient, &LocalAddr, &PeerAddr, &UdpIo)>();

    let result = query.single(app.world());
    assert!(result.is_ok(), "Client entity should exist");
    let (_, _, local_addr, peer_addr, _) = result.unwrap();
    assert_eq!(local_addr.0, config.client_addr);
    assert_eq!(peer_addr.0, config.server_addr);
}

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
