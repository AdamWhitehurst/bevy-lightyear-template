use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use nostr_client::{
    RelayPool, ServerAnnouncement, ServerIdentity, SERVER_ANNOUNCEMENT_REPUBLISH_SECS,
    SERVER_ANNOUNCEMENT_VERSION,
};
use server_lightyear::ServerNetworkConfig;

pub struct ServerAnnouncementPlugin;

impl Plugin for ServerAnnouncementPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(protocol::AppState::Ready),
            publish_announcement_on_ready,
        )
        .add_systems(
            Update,
            publish_announcement_periodically.run_if(in_state(protocol::AppState::Ready)),
        );
    }
}

fn publish_announcement_on_ready(
    pool: Res<RelayPool>,
    identity: Res<ServerIdentity>,
    network: Res<ServerNetworkConfig>,
) {
    publish_announcement(&pool, &identity, &network);
}

fn publish_announcement_periodically(
    time: Res<Time>,
    mut last_publish: Local<Option<f64>>,
    pool: Res<RelayPool>,
    identity: Res<ServerIdentity>,
    network: Res<ServerNetworkConfig>,
) {
    let now = time.elapsed_secs_f64();
    let Some(last) = *last_publish else {
        *last_publish = Some(now);
        return;
    };

    if now - last < SERVER_ANNOUNCEMENT_REPUBLISH_SECS as f64 {
        return;
    }

    *last_publish = Some(now);
    publish_announcement(&pool, &identity, &network);
}

fn publish_announcement(
    pool: &RelayPool,
    identity: &ServerIdentity,
    network: &ServerNetworkConfig,
) {
    let client = pool.client.clone();
    let identity = identity.clone();
    let announcement = ServerAnnouncement {
        server_addr: std::net::SocketAddr::from((network.bind_addr, network.port)),
        cert_digest: load_cert_digest(),
        display_name: "Untitled Brawler Server".to_string(),
        version: SERVER_ANNOUNCEMENT_VERSION,
    };

    IoTaskPool::get()
        .spawn(async move {
            match nostr_client::announcement::publish_server_announcement(
                client,
                identity,
                announcement,
            )
            .await
            {
                Ok(event_id) => info!(%event_id, "published Nostr server announcement"),
                Err(error) => panic!("failed to publish Nostr server announcement: {error}"),
            }
        })
        .detach();
}

fn load_cert_digest() -> String {
    include_str!("../../../certificates/digest.txt")
        .trim()
        .to_string()
}
