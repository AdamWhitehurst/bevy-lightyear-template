use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use super::lod::generate_lod_meshes;

/// A loaded `.vox` model with LOD mesh sub-assets.
///
/// LOD 0 is full resolution. Each subsequent level halves resolution via 2x2x2 downsampling.
/// Mesh handles are labeled sub-assets: `"mesh_lod0"`, `"mesh_lod1"`, etc.
#[derive(Asset, TypePath)]
pub struct VoxModelAsset {
    /// LOD mesh handles, index 0 = full resolution.
    pub lod_meshes: Vec<Handle<Mesh>>,
    /// Model dimensions in voxels (Bevy Y-up space).
    pub size: UVec3,
}

/// Custom asset loader for `.vox` files.
///
/// Parses via `dot_vox`, generates greedy-meshed Bevy `Mesh` with vertex colors,
/// and produces multiple LOD levels as labeled sub-assets.
#[derive(Default, TypePath)]
pub(super) struct VoxModelLoader;

impl AssetLoader for VoxModelLoader {
    type Asset = VoxModelAsset;
    type Settings = ();
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn extensions(&self) -> &[&str] {
        &["vox"]
    }

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        load_context: &mut LoadContext<'_>,
    ) -> Result<VoxModelAsset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let data = dot_vox::load_bytes(&bytes).map_err(|e| format!("dot_vox parse error: {e}"))?;
        let model = &data.models[0];

        let lod_meshes = generate_lod_meshes(model, &data.palette, load_context);

        let size = UVec3::new(model.size.x, model.size.z, model.size.y); // Z-up → Y-up

        Ok(VoxModelAsset { lod_meshes, size })
    }
}
