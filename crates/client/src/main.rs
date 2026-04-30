pub mod diagnostics;
pub mod gameplay;
pub mod map;
pub mod transition;
pub mod world_object;

use bevy::prelude::*;
use client::persistence::fs_encrypted_identity::FsEncryptedIdentityStore;
use client_lightyear::{ClientNetworkConfig, ClientNetworkPlugin};
use dev::DevPlugin;
use diagnostics::ClientDiagnosticsPlugin;
use gameplay::ClientGameplayPlugin;
use lightyear::prelude::client::*;
use map::ClientMapPlugin;
use nostr_client::{
    EncryptedIdentity, LoginError, NostrClientConfig, NostrClientPlugin, SaveEncryptedIdentity,
    StoredEncryptedIdentity,
};
use persistence::{PendingStoreOps, StoreBackend};
use protocol::diagnostics::SharedDiagnosticsPlugin;
use protocol::*;
use render::RenderPlugin;
use std::{path::PathBuf, sync::Arc, time::Duration};
use ui::{UiClientConfig, UiPlugin};

fn main() {
    let client_id = parse_client_id();

    let network_config = ClientNetworkConfig {
        client_id,
        certificate_digest: include_str!("../../../certificates/digest.txt")
            .trim()
            .to_string(),
        ..Default::default()
    };

    // Create UI config from network config to keep them in sync
    let ui_config = UiClientConfig {
        server_addr: network_config.server_addr,
        client_id: network_config.client_id,
        certificate_digest: network_config.certificate_digest.clone(),
        protocol_id: network_config.protocol_id,
        private_key: network_config.private_key,
    };

    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin {
            file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets").to_string(),
            ..default()
        }))
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(SharedGameplayPlugin)
        .add_plugins(NostrClientPlugin {
            config: NostrClientConfig {
                mark_identity_load_complete_on_startup: false,
                ..Default::default()
            },
        })
        .add_plugins(ClientNetworkPlugin {
            config: network_config,
        })
        .add_message::<SaveEncryptedIdentity>()
        .init_resource::<StoredEncryptedIdentity>()
        .init_resource::<LoginError>()
        .add_systems(Startup, spawn_identity_store)
        .add_systems(
            Update,
            (poll_identity_store_load, handle_identity_save_requests),
        )
        .insert_resource(ui_config) // Override default UiClientConfig
        .add_plugins(ClientGameplayPlugin)
        .add_plugins(ClientMapPlugin)
        .add_plugins(transition::ClientTransitionPlugin)
        .add_plugins(RenderPlugin)
        .add_plugins(UiPlugin)
        .add_plugins(DevPlugin)
        .add_plugins(SharedDiagnosticsPlugin)
        .add_plugins(ClientDiagnosticsPlugin)
        .run();
}

fn parse_client_id() -> u64 {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "-c" || args[i] == "--client-id" {
            if let Some(id_str) = args.get(i + 1) {
                return id_str.parse().expect("Invalid client ID");
            }
        }
    }
    0
}

fn spawn_identity_store(mut commands: Commands) {
    let store = FsEncryptedIdentityStore {
        base_dir: Arc::new(PathBuf::from("worlds")),
    };
    let mut ops = PendingStoreOps::<(), EncryptedIdentity>::default();
    ops.spawn_load(&store, ());
    commands.spawn((
        Name::new("Encrypted Identity Store"),
        StoreBackend::new(store),
        ops,
    ));
}

fn poll_identity_store_load(
    mut query: Query<(
        &StoreBackend<(), EncryptedIdentity, FsEncryptedIdentityStore>,
        &mut PendingStoreOps<(), EncryptedIdentity>,
    )>,
    mut stored: ResMut<StoredEncryptedIdentity>,
    mut complete: ResMut<IdentityLoadComplete>,
) {
    let Ok((_store, mut ops)) = query.single_mut() else {
        trace!("poll_identity_store_load: identity store entity not spawned yet");
        return;
    };

    ops.poll();
    for (_key, loaded) in ops.completed_loads.drain(..) {
        stored.0 = loaded;
        complete.0 = true;
        info!("Encrypted identity load complete");
    }
    for (_key, error) in ops.load_errors.drain(..) {
        panic!("Failed to load encrypted identity: {error}");
    }
}

fn handle_identity_save_requests(
    mut requests: MessageReader<SaveEncryptedIdentity>,
    mut query: Query<(
        &StoreBackend<(), EncryptedIdentity, FsEncryptedIdentityStore>,
        &mut PendingStoreOps<(), EncryptedIdentity>,
    )>,
) {
    let Ok((store, mut ops)) = query.single_mut() else {
        trace!("handle_identity_save_requests: identity store entity not spawned yet");
        return;
    };

    for request in requests.read() {
        ops.spawn_save(&store.0, (), request.0.clone());
    }
}
