use avian3d::prelude::ColliderConstructor;
use bevy::prelude::*;

use super::loader::WorldObjectLoader;
use super::loading::{insert_world_object_defs, load_world_object_defs, reload_world_object_defs};
use super::registry::WorldObjectDefRegistry;
use super::types::{ObjectCategory, VisualKind, WorldObjectDef};
use crate::app_state::AppState;
use crate::Health;

#[cfg(target_arch = "wasm32")]
use {super::registry::WorldObjectManifest, bevy_common_assets::ron::RonAssetPlugin};

/// Loads and hot-reloads world object definitions from `.object.ron` files.
///
/// Follows the ability system loading pattern:
/// - Native: `load_folder("objects")` → aggregated into `WorldObjectDefRegistry`
/// - WASM: manifest → individual loads → aggregated into `WorldObjectDefRegistry`
pub struct WorldObjectPlugin;

impl Plugin for WorldObjectPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<WorldObjectDef>();
        app.init_asset_loader::<WorldObjectLoader>();

        #[cfg(target_arch = "wasm32")]
        app.add_plugins(RonAssetPlugin::<WorldObjectManifest>::new(&[
            "objects.manifest.ron",
        ]));

        app.add_systems(Startup, load_world_object_defs);

        #[cfg(target_arch = "wasm32")]
        app.add_systems(
            PreUpdate,
            super::loading::trigger_individual_object_loads
                .run_if(in_state(crate::app_state::AppState::Loading)),
        );

        app.add_systems(
            Update,
            insert_world_object_defs.run_if(not(resource_exists::<WorldObjectDefRegistry>)),
        );
        app.add_systems(
            Update,
            reload_world_object_defs.run_if(in_state(AppState::Ready)),
        );

        // Register types for RON reflect-based component deserialization.
        app.register_type::<Health>();
        app.register_type::<ObjectCategory>();
        app.register_type::<VisualKind>();
        app.register_type::<ColliderConstructor>();
    }
}
