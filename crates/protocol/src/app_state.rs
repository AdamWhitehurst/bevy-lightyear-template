use bevy::asset::UntypedHandle;
use bevy::prelude::*;

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, States)]
pub enum AppState {
    #[default]
    Loading,
    Ready,
}

/// Collects asset handles that must finish loading before transitioning to `AppState::Ready`.
#[derive(Resource, Default)]
pub struct TrackedAssets(Vec<UntypedHandle>);

#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct RelayPoolReady(pub bool);

#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct IdentityLoadComplete(pub bool);

impl TrackedAssets {
    pub fn add(&mut self, handle: impl Into<UntypedHandle>) {
        self.0.push(handle.into());
    }
}

pub struct AppStatePlugin;

impl Plugin for AppStatePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>();
        app.init_resource::<TrackedAssets>();
        app.init_resource::<RelayPoolReady>();
        app.init_resource::<IdentityLoadComplete>();
        app.add_systems(
            Update,
            check_assets_loaded.run_if(in_state(AppState::Loading)),
        );
    }
}

fn check_assets_loaded(
    asset_server: Res<AssetServer>,
    tracked: Res<TrackedAssets>,
    relay_ready: Res<RelayPoolReady>,
    identity_ready: Res<IdentityLoadComplete>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let assets_loaded = tracked
        .0
        .iter()
        .all(|handle| asset_server.is_loaded_with_dependencies(handle));

    if !assets_loaded {
        trace!("check_assets_loaded: tracked assets still loading");
        return;
    }

    if !relay_ready.0 {
        trace!("check_assets_loaded: waiting for Nostr relay EOSE");
        return;
    }

    if !identity_ready.0 {
        trace!("check_assets_loaded: waiting for identity store load");
        return;
    }

    info!("Startup gates complete, transitioning to AppState::Ready");
    next_state.set(AppState::Ready);
}
