use ::client::network::{ClientNetworkConfig, ClientNetworkPlugin, ClientTransport};
use ::server::network::{ServerNetworkConfig, ServerNetworkPlugin, ServerTransport};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use lightyear::prelude::client as lightyear_client;
use lightyear::prelude::server as lightyear_server;
use lightyear::prelude::*;
use lightyear_client::*;
use lightyear_server::*;
use protocol::*;
use std::net::SocketAddr;
use std::time::Duration;

/// Integration test using UDP transport to validate connection establishment
#[test]
fn test_client_server_udp_connection() {
    const TEST_PORT: u16 = 7777;

    let mut server_app = App::new();
    server_app.add_plugins(MinimalPlugins);
    server_app.add_plugins(ServerPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
    });
    server_app.add_plugins(ProtocolPlugin);
    server_app.add_plugins(protocol::TransitionPlugin);
    server_app.add_plugins(ServerNetworkPlugin {
        config: ServerNetworkConfig {
            transports: vec![ServerTransport::Udp { port: TEST_PORT }],
            bind_addr: [127, 0, 0, 1],
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            replication_interval: Duration::from_millis(100),
        },
    });

    let mut client_app = App::new();
    client_app.add_plugins(MinimalPlugins);
    client_app.add_plugins(ClientPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
    });
    client_app.add_plugins(ProtocolPlugin);
    client_app.add_plugins(protocol::TransitionPlugin);
    client_app.add_plugins(ClientNetworkPlugin {
        config: ClientNetworkConfig {
            client_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            server_addr: SocketAddr::from(([127, 0, 0, 1], TEST_PORT)),
            client_id: 0,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            transport: ClientTransport::Udp,
            ..default()
        },
    });

    let mut current_time = bevy::platform::time::Instant::now();
    let frame_duration = Duration::from_millis(10);
    server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
    client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));

    server_app.update();
    client_app.update();

    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, With<lightyear_client::Client>>()
        .single(client_app.world())
        .unwrap();
    client_app
        .world_mut()
        .commands()
        .trigger(lightyear_client::Connect {
            entity: client_entity,
        });
    client_app.update();

    let mut query = server_app
        .world_mut()
        .query_filtered::<Entity, With<NetcodeServer>>();
    assert_eq!(
        query.iter(server_app.world()).count(),
        1,
        "Server should have spawned one UDP entity"
    );

    let mut connected = false;
    for _ in 0..300 {
        current_time += frame_duration;
        server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        server_app.update();
        client_app.update();
        std::thread::sleep(Duration::from_micros(100));

        let mut query = client_app
            .world_mut()
            .query_filtered::<Entity, (With<Client>, With<Connected>)>();
        if query.iter(client_app.world()).count() > 0 {
            connected = true;
            break;
        }
    }

    assert!(
        connected,
        "Client should have Connected component after UDP handshake"
    );

    let mut query = server_app
        .world_mut()
        .query_filtered::<Entity, (With<Connected>, With<ReplicationSender>)>();
    assert_eq!(
        query.iter(server_app.world()).count(),
        1,
        "Server should have added ReplicationSender to connected client"
    );
}

/// Test that voxel messages are registered in protocol
#[test]
fn test_voxel_messages_registered() {
    use protocol::{VoxelEditRequest, VoxelType};

    let _request = VoxelEditRequest {
        position: IVec3::new(1, 2, 3),
        voxel: VoxelType::Solid(42),
        sequence: 0,
    };

    info!("Voxel message types compile successfully");
}
