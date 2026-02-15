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
        app.add_systems(
            Update,
            check_assets_loaded.run_if(in_state(AppState::Loading)),
        );
    }
}

fn check_assets_loaded(
    asset_server: Res<AssetServer>,
    tracked: Res<TrackedAssets>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let all_loaded = tracked
        .0
        .iter()
        .all(|handle| asset_server.is_loaded_with_dependencies(handle));

    if all_loaded {
        info!("All tracked assets loaded, transitioning to AppState::Ready");
        next_state.set(AppState::Ready);
    }
}
