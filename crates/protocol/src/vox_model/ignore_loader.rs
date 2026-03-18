use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::reflect::TypePath;

/// Placeholder asset for file types in the `models/` directory that have no real loader.
///
/// Registered so that `load_folder("models")` does not fail when encountering
/// `.mtl`, `.obj`, or `.png` files alongside `.vox` files. These handles are
/// filtered out by `try_typed::<VoxModelAsset>()` when building the registry.
#[derive(Asset, TypePath)]
pub(super) struct IgnoredModelAsset;

/// No-op asset loader that produces [`IgnoredModelAsset`] for non-vox file types.
#[derive(Default, TypePath)]
pub(super) struct IgnoredModelAssetLoader;

impl AssetLoader for IgnoredModelAssetLoader {
    type Asset = IgnoredModelAsset;
    type Settings = ();
    type Error = std::convert::Infallible;

    fn extensions(&self) -> &[&str] {
        &["mtl", "obj", "png"]
    }

    async fn load(
        &self,
        _reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<IgnoredModelAsset, Self::Error> {
        Ok(IgnoredModelAsset)
    }
}
