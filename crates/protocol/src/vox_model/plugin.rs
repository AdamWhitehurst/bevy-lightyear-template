use bevy::prelude::*;

use super::ignore_loader::{IgnoredModelAsset, IgnoredModelAssetLoader};
use super::loader::{VoxModelAsset, VoxModelLoader};
use super::loading::{insert_vox_model_registry, load_vox_models, VoxModelRegistry};
use crate::app_state::AppState;

#[cfg(target_arch = "wasm32")]
use {super::manifest::VoxModelManifest, bevy_common_assets::ron::RonAssetPlugin};

/// Loads and hot-reloads `.vox` model assets with LOD mesh generation.
///
/// Follows the world object loading pattern:
/// - Native: `load_folder("models")` → aggregated into `VoxModelRegistry`
/// - WASM: manifest → individual loads → aggregated into `VoxModelRegistry`
///
/// Registers [`IgnoredModelAssetLoader`] for `.mtl`/`.obj` so `load_folder` does not
/// fail on non-vox files in the `models/` directory.
pub struct VoxModelPlugin;

impl Plugin for VoxModelPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<VoxModelAsset>();
        app.init_asset_loader::<VoxModelLoader>();

        // No-op loader so load_folder("models") skips .mtl/.obj files instead of failing.
        app.init_asset::<IgnoredModelAsset>();
        app.init_asset_loader::<IgnoredModelAssetLoader>();

        #[cfg(target_arch = "wasm32")]
        app.add_plugins(RonAssetPlugin::<VoxModelManifest>::new(&[
            "models.manifest.ron",
        ]));

        app.add_systems(Startup, load_vox_models);

        #[cfg(target_arch = "wasm32")]
        app.add_systems(
            PreUpdate,
            super::loading::trigger_individual_model_loads
                .run_if(not(resource_exists::<VoxModelRegistry>)),
        );

        app.add_systems(
            Update,
            insert_vox_model_registry.run_if(not(resource_exists::<VoxModelRegistry>)),
        );

        #[cfg(not(target_arch = "wasm32"))]
        app.add_systems(
            Update,
            super::loading::reload_vox_models
                .run_if(in_state(AppState::Ready).and(resource_exists::<VoxModelRegistry>)),
        );
    }
}
