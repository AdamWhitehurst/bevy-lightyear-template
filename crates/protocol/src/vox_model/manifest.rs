use bevy::prelude::*;
use bevy::reflect::TypePath;
use serde::Deserialize;

/// Lists vox model paths for WASM builds (where `load_folder` is unavailable).
///
/// Must be updated manually when `.vox` files are added or removed —
/// the same convention as `objects.manifest.ron`.
#[derive(Deserialize, Asset, TypePath)]
pub struct VoxModelManifest(pub Vec<String>);
