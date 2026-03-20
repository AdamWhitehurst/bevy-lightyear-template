use bevy::prelude::*;

use super::loader::TerrainDefLoader;
use super::loading::{insert_terrain_defs, load_terrain_defs};
use super::registry::TerrainDefRegistry;
use super::types::TerrainDef;

#[cfg(target_arch = "wasm32")]
use {super::registry::TerrainManifest, bevy_common_assets::ron::RonAssetPlugin};

/// Loads terrain definitions from `.terrain.ron` files.
///
/// Follows the world object loading pattern:
/// - Native: `load_folder("terrain")` aggregated into `TerrainDefRegistry`
/// - WASM: manifest then individual loads aggregated into `TerrainDefRegistry`
pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<TerrainDef>();
        app.init_asset_loader::<TerrainDefLoader>();

        #[cfg(target_arch = "wasm32")]
        app.add_plugins(RonAssetPlugin::<TerrainManifest>::new(&[
            "terrain.manifest.ron",
        ]));

        app.add_systems(Startup, load_terrain_defs);

        #[cfg(target_arch = "wasm32")]
        app.add_systems(
            PreUpdate,
            super::loading::trigger_individual_terrain_loads
                .run_if(in_state(crate::app_state::AppState::Loading)),
        );

        app.add_systems(
            Update,
            insert_terrain_defs.run_if(not(resource_exists::<TerrainDefRegistry>)),
        );
    }
}
