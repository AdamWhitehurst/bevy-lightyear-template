use bevy::prelude::*;
use std::collections::HashMap;

use super::loader::VoxModelAsset;
use crate::app_state::TrackedAssets;

#[cfg(not(target_arch = "wasm32"))]
use bevy::asset::LoadedFolder;

#[cfg(target_arch = "wasm32")]
use super::manifest::VoxModelManifest;

/// Holds the folder handle returned by `load_folder("models")` (native only).
/// Kept alive to prevent asset unloading; also used to enumerate loaded models.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
pub(super) struct ModelFolderHandle(pub Handle<LoadedFolder>);

/// Holds the manifest handle (WASM only).
/// Once loaded, the trigger system reads paths and starts individual loads.
#[cfg(target_arch = "wasm32")]
#[derive(Resource)]
pub(super) struct ModelManifestHandle(pub Handle<VoxModelManifest>);

/// Accumulates individual model handles loaded from the manifest (WASM only).
/// Each handle is also added to `TrackedAssets` for load-gating.
#[cfg(target_arch = "wasm32")]
#[derive(Resource, Default)]
pub(super) struct PendingModelHandles(pub Vec<Handle<VoxModelAsset>>);

/// All loaded vox models, keyed by asset path relative to `assets/` (e.g. `"models/trees/tree_circle.vox"`).
///
/// Populated during `AppState::Loading` via `VoxModelPlugin` systems.
/// Available to both server and client after `AppState::Ready`.
#[derive(Resource, Clone, Debug)]
pub struct VoxModelRegistry {
    pub models: HashMap<String, Handle<VoxModelAsset>>,
}

impl VoxModelRegistry {
    /// Looks up a vox model asset handle by path.
    pub fn get(&self, path: &str) -> Option<&Handle<VoxModelAsset>> {
        self.models.get(path)
    }

    /// Resolves the full-resolution (LOD 0) mesh for a vox model by asset path.
    ///
    /// Two-step lookup: path → `VoxModelAsset` → `lod_meshes[0]` → `&Mesh`.
    pub fn get_lod0_mesh<'a>(
        &self,
        path: &str,
        vox_assets: &Assets<VoxModelAsset>,
        meshes: &'a Assets<Mesh>,
    ) -> Option<&'a Mesh> {
        let handle = self.models.get(path)?;
        let asset = vox_assets.get(handle)?;
        let mesh_handle = asset.lod_meshes.first()?;
        meshes.get(mesh_handle)
    }
}

/// Starts loading all model assets at app startup via `load_folder` (native).
///
/// Non-vox files (`.mtl`, `.obj`) are handled by [`IgnoredModelAssetLoader`](super::ignore_loader::IgnoredModelAssetLoader)
/// so `load_folder` does not fail. The resulting handles are filtered by type
/// in [`insert_vox_model_registry`].
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn load_vox_models(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<TrackedAssets>,
) {
    let handle = asset_server.load_folder("models");
    tracked.add(handle.clone());
    commands.insert_resource(ModelFolderHandle(handle));
}

/// Starts loading the vox model manifest at app startup (WASM).
#[cfg(target_arch = "wasm32")]
pub(super) fn load_vox_models(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<TrackedAssets>,
) {
    let handle = asset_server.load::<VoxModelManifest>("models.manifest.ron");
    tracked.add(handle.clone());
    commands.insert_resource(ModelManifestHandle(handle));
    commands.init_resource::<PendingModelHandles>();
}

/// Loads individual `.vox` files once the manifest is ready (WASM only).
#[cfg(target_arch = "wasm32")]
pub(super) fn trigger_individual_model_loads(
    manifest_handle: Option<Res<ModelManifestHandle>>,
    manifests: Res<Assets<VoxModelManifest>>,
    mut pending: ResMut<PendingModelHandles>,
    mut tracked: ResMut<TrackedAssets>,
    asset_server: Res<AssetServer>,
) {
    if !pending.0.is_empty() {
        trace!("Individual model loads already triggered");
        return;
    }
    let Some(manifest_handle) = manifest_handle else {
        trace!("Model manifest handle not yet available");
        return;
    };
    let Some(manifest) = manifests.get(&manifest_handle.0) else {
        trace!("Model manifest asset not yet loaded");
        return;
    };
    for path in &manifest.0 {
        let handle = asset_server.load::<VoxModelAsset>(format!("models/{path}"));
        tracked.add(handle.clone());
        pending.0.push(handle);
    }
}

/// Builds the `VoxModelRegistry` once all model assets are loaded (native).
///
/// Runs only while the registry does not yet exist (gated in plugin via `run_if`).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn insert_vox_model_registry(
    mut commands: Commands,
    folder_handle: Res<ModelFolderHandle>,
    loaded_folders: Res<Assets<LoadedFolder>>,
    asset_server: Res<AssetServer>,
) {
    let Some(folder) = loaded_folders.get(&folder_handle.0) else {
        trace!("Model folder not yet loaded");
        return;
    };
    let models = collect_model_handles(
        folder
            .handles
            .iter()
            .filter_map(|h| h.clone().try_typed::<VoxModelAsset>().ok()),
        &asset_server,
    );
    info!("Loaded {} vox models", models.len());
    commands.insert_resource(VoxModelRegistry { models });
}

/// Builds the `VoxModelRegistry` once all model assets are loaded (WASM).
///
/// Runs only while the registry does not yet exist (gated in plugin via `run_if`).
#[cfg(target_arch = "wasm32")]
pub(super) fn insert_vox_model_registry(
    mut commands: Commands,
    pending: Res<PendingModelHandles>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    asset_server: Res<AssetServer>,
) {
    if pending.0.is_empty() {
        trace!("No pending model handles yet");
        return;
    }
    if pending.0.iter().any(|h| vox_assets.get(h).is_none()) {
        trace!("Not all vox model assets loaded yet");
        return;
    }
    let models = collect_model_handles(pending.0.iter().cloned(), &asset_server);
    info!("Loaded {} vox models", models.len());
    commands.insert_resource(VoxModelRegistry { models });
}

/// Rebuilds `VoxModelRegistry` when any vox model is hot-reloaded (native only).
///
/// Runs only in `AppState::Ready`, which guarantees the registry and folder exist.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn reload_vox_models(
    mut events: MessageReader<AssetEvent<VoxModelAsset>>,
    folder_handle: Res<ModelFolderHandle>,
    loaded_folders: Res<Assets<LoadedFolder>>,
    asset_server: Res<AssetServer>,
    mut registry: ResMut<VoxModelRegistry>,
) {
    if !events
        .read()
        .any(|e| matches!(e, AssetEvent::Modified { .. }))
    {
        trace!("No modified events found for reload_vox_models");
        return;
    }
    let folder = loaded_folders
        .get(&folder_handle.0)
        .expect("No loaded folder found in reload_vox_models");

    registry.models = collect_model_handles(
        folder
            .handles
            .iter()
            .filter_map(|h| h.clone().try_typed::<VoxModelAsset>().ok()),
        &asset_server,
    );
    trace!("Hot-reloaded {} vox models", registry.models.len());
}

/// Collects model handles into a path-to-handle map.
fn collect_model_handles(
    handles: impl Iterator<Item = Handle<VoxModelAsset>>,
    asset_server: &AssetServer,
) -> HashMap<String, Handle<VoxModelAsset>> {
    let mut models = HashMap::new();
    for handle in handles {
        let path = asset_server
            .get_path(handle.id())
            .expect("Handle<VoxModelAsset> to have file path after loading");

        models.insert(path.path().to_string_lossy().into_owned(), handle);
    }
    models
}
