#[cfg(target_arch = "wasm32")]
use super::types::AbilityManifest;
use super::types::{AbilityAsset, AbilityDefs, AbilityId, AbilitySlots};
use crate::app_state::TrackedAssets;
use bevy::prelude::*;
use std::collections::HashMap;

#[cfg(not(target_arch = "wasm32"))]
use bevy::asset::LoadedFolder;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
pub(super) struct AbilityFolderHandle(pub(super) Handle<LoadedFolder>);

#[cfg(target_arch = "wasm32")]
#[derive(Resource)]
pub(super) struct AbilityManifestHandle(pub(super) Handle<AbilityManifest>);

#[cfg(target_arch = "wasm32")]
#[derive(Resource)]
pub(super) struct PendingAbilityHandles(pub(super) Vec<Handle<AbilityAsset>>);

/// Internal handle for the default ability slots asset — used only for loading and hot-reload.
#[derive(Resource)]
pub(super) struct DefaultAbilitySlotsHandle(pub(super) Handle<AbilitySlots>);

/// The resolved global default ability slots, populated once the asset finishes loading.
#[derive(Resource, Clone, Default)]
pub struct DefaultAbilitySlots(pub AbilitySlots);

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn load_ability_defs(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<TrackedAssets>,
) {
    let handle = asset_server.load_folder("abilities");
    tracked.add(handle.clone());
    commands.insert_resource(AbilityFolderHandle(handle));
}

#[cfg(target_arch = "wasm32")]
pub(super) fn load_ability_defs(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<TrackedAssets>,
) {
    let handle = asset_server.load::<AbilityManifest>("abilities.manifest.ron");
    tracked.add(handle.clone());
    commands.insert_resource(AbilityManifestHandle(handle));
}

#[cfg(target_arch = "wasm32")]
pub(super) fn trigger_individual_ability_loads(
    manifest_handle: Option<Res<AbilityManifestHandle>>,
    manifest_assets: Res<Assets<AbilityManifest>>,
    pending: Option<Res<PendingAbilityHandles>>,
    mut tracked: ResMut<TrackedAssets>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    if pending.is_some() {
        trace!("PendingAbilityHandles already exists");
        return;
    }
    let Some(manifest_handle) = manifest_handle else {
        trace!("ability manifest handle not yet loaded");
        return;
    };
    let Some(manifest) = manifest_assets.get(&manifest_handle.0) else {
        trace!("ability manifest asset not yet available");
        return;
    };
    let handles: Vec<Handle<AbilityAsset>> = manifest
        .0
        .iter()
        .map(|id| {
            let h: Handle<AbilityAsset> = asset_server.load(format!("abilities/{id}.ability.ron"));
            tracked.add(h.clone());
            h
        })
        .collect();
    commands.insert_resource(PendingAbilityHandles(handles));
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn insert_ability_defs(
    mut commands: Commands,
    folder_handle: Option<Res<AbilityFolderHandle>>,
    loaded_folders: Res<Assets<LoadedFolder>>,
    asset_server: Res<AssetServer>,
    existing: Option<Res<AbilityDefs>>,
) {
    if existing.is_some() {
        trace!("AbilityDefs already inserted");
        return;
    }
    let Some(folder_handle) = folder_handle else {
        trace!("ability folder handle not yet loaded");
        return;
    };
    let Some(folder) = loaded_folders.get(&folder_handle.0) else {
        trace!("ability folder not yet available in Assets<LoadedFolder>");
        return;
    };
    let abilities = collect_ability_handles_from_folder(folder, &asset_server);
    info!("Loaded {} ability definitions", abilities.len());
    commands.insert_resource(AbilityDefs { abilities });
}

#[cfg(target_arch = "wasm32")]
pub(super) fn insert_ability_defs(
    mut commands: Commands,
    pending: Option<Res<PendingAbilityHandles>>,
    ability_assets: Res<Assets<AbilityAsset>>,
    asset_server: Res<AssetServer>,
    existing: Option<Res<AbilityDefs>>,
) {
    if existing.is_some() {
        trace!("AbilityDefs already inserted");
        return;
    }
    let Some(pending) = pending else {
        trace!("PendingAbilityHandles not yet available");
        return;
    };
    let abilities: HashMap<AbilityId, Handle<AbilityAsset>> = pending
        .0
        .iter()
        .filter_map(|handle| {
            // Verify asset is loaded before including
            ability_assets.get(handle)?;
            let path = asset_server.get_path(handle.id())?;
            let id = ability_id_from_path(&path)?;
            Some((id, handle.clone()))
        })
        .collect();
    if abilities.len() != pending.0.len() {
        trace!(
            "not all ability assets loaded yet ({}/{})",
            abilities.len(),
            pending.0.len()
        );
        return;
    }
    info!("Loaded {} ability definitions", abilities.len());
    commands.insert_resource(AbilityDefs { abilities });
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn reload_ability_defs(
    mut commands: Commands,
    folder_handle: Option<Res<AbilityFolderHandle>>,
    loaded_folders: Res<Assets<LoadedFolder>>,
    asset_server: Res<AssetServer>,
    mut events: MessageReader<AssetEvent<AbilityAsset>>,
) {
    let Some(folder_handle) = folder_handle else {
        // Folder handle not yet loaded during startup — drain events to avoid stale backlog
        events.clear();
        return;
    };
    let has_changes = events
        .read()
        .any(|e| matches!(e, AssetEvent::Modified { .. }));
    if !has_changes {
        // No ability asset modifications this frame
        return;
    }
    let Some(folder) = loaded_folders.get(&folder_handle.0) else {
        warn!("ability assets changed but LoadedFolder not available");
        return;
    };
    let abilities = collect_ability_handles_from_folder(folder, &asset_server);
    info!("Hot-reloaded {} ability definitions", abilities.len());
    commands.insert_resource(AbilityDefs { abilities });
}

#[cfg(target_arch = "wasm32")]
pub(super) fn reload_ability_defs(
    mut commands: Commands,
    pending: Option<Res<PendingAbilityHandles>>,
    ability_assets: Res<Assets<AbilityAsset>>,
    asset_server: Res<AssetServer>,
    mut events: MessageReader<AssetEvent<AbilityAsset>>,
) {
    let Some(pending) = pending else {
        // Pending handles not yet available during startup — drain events to avoid stale backlog
        events.clear();
        return;
    };
    let has_changes = events
        .read()
        .any(|e| matches!(e, AssetEvent::Modified { .. }));
    if !has_changes {
        // No ability asset modifications this frame
        return;
    }
    let abilities: HashMap<AbilityId, Handle<AbilityAsset>> = pending
        .0
        .iter()
        .filter_map(|handle| {
            ability_assets.get(handle)?;
            let path = asset_server.get_path(handle.id())?;
            let id = ability_id_from_path(&path)?;
            Some((id, handle.clone()))
        })
        .collect();
    if abilities.len() != pending.0.len() {
        trace!(
            "not all ability assets loaded yet for reload ({}/{})",
            abilities.len(),
            pending.0.len()
        );
        return;
    }
    info!("Hot-reloaded {} ability definitions", abilities.len());
    commands.insert_resource(AbilityDefs { abilities });
}

fn ability_id_from_path(path: &bevy::asset::AssetPath) -> Option<AbilityId> {
    let name = path.path().file_name()?.to_str()?;
    Some(AbilityId(name.strip_suffix(".ability.ron")?.to_string()))
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_ability_handles_from_folder(
    folder: &LoadedFolder,
    asset_server: &AssetServer,
) -> HashMap<AbilityId, Handle<AbilityAsset>> {
    folder
        .handles
        .iter()
        .filter_map(|handle| {
            let path = asset_server.get_path(handle.id())?;
            let name = path.path().file_name()?.to_str()?;
            if !name.ends_with(".ability.ron") {
                return None;
            }
            let id = ability_id_from_path(&path)?;
            let typed = handle.clone().typed::<AbilityAsset>();
            Some((id, typed))
        })
        .collect()
}

pub(super) fn load_default_ability_slots(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<TrackedAssets>,
) {
    let handle = asset_server.load::<AbilitySlots>("default.ability_slots.ron");
    tracked.add(handle.clone());
    commands.insert_resource(DefaultAbilitySlotsHandle(handle));
}

pub(super) fn sync_default_ability_slots(
    mut commands: Commands,
    handle: Option<Res<DefaultAbilitySlotsHandle>>,
    ability_slots_assets: Res<Assets<AbilitySlots>>,
    mut events: MessageReader<AssetEvent<AbilitySlots>>,
) {
    let Some(handle) = handle else {
        events.clear();
        return;
    };
    let id = handle.0.id();
    let is_relevant = |e: &AssetEvent<AbilitySlots>| {
        matches!(e,
            AssetEvent::LoadedWithDependencies { id: eid } |
            AssetEvent::Modified { id: eid }
            if *eid == id
        )
    };
    if !events.read().any(is_relevant) {
        return;
    }
    let Some(slots) = ability_slots_assets.get(&handle.0) else {
        warn!("default.ability_slots.ron event fired but asset not available");
        return;
    };
    info!("Synced default ability slots");
    commands.insert_resource(DefaultAbilitySlots(slots.clone()));
}
