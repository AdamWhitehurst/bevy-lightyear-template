use bevy::prelude::*;
use protocol::{IdentityLoadComplete, RelayPoolReady};

use crate::announcement::{
    poll_server_announcements, spawn_server_announcement_subscription,
    ServerAnnouncementSubscriptionStarted, ServerList,
};
use crate::relay_pool::{poll_relay_pool_ready, shutdown_relay_pool, spawn_relay_pool};

#[derive(Clone, Resource, Debug)]
pub struct NostrClientConfig {
    pub relays: Vec<String>,
    pub mark_identity_load_complete_on_startup: bool,
}

impl Default for NostrClientConfig {
    fn default() -> Self {
        Self {
            relays: relays_from_env_or_default(),
            mark_identity_load_complete_on_startup: true,
        }
    }
}

fn relays_from_env_or_default() -> Vec<String> {
    std::env::var("NOSTR_RELAYS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|relay| !relay.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .filter(|relays: &Vec<String>| !relays.is_empty())
        .unwrap_or_else(|| {
            vec![
                "wss://relay.damus.io".to_string(),
                "wss://nos.lol".to_string(),
                "wss://relay.primal.net".to_string(),
            ]
        })
}

#[derive(Default)]
pub struct NostrClientPlugin {
    pub config: NostrClientConfig,
}

impl Plugin for NostrClientPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config.clone())
            .init_resource::<RelayPoolReady>()
            .init_resource::<IdentityLoadComplete>()
            .init_resource::<ServerList>()
            .init_resource::<ServerAnnouncementSubscriptionStarted>()
            .add_systems(Startup, (mark_identity_load_complete, spawn_relay_pool))
            .add_systems(
                Update,
                (
                    poll_relay_pool_ready,
                    spawn_server_announcement_subscription,
                    poll_server_announcements,
                ),
            )
            .add_systems(Last, shutdown_relay_pool);
    }
}

fn mark_identity_load_complete(
    mut complete: ResMut<IdentityLoadComplete>,
    config: Res<NostrClientConfig>,
) {
    if config.mark_identity_load_complete_on_startup {
        complete.0 = true;
        debug!("identity load marked complete by Nostr client config");
    }
}
