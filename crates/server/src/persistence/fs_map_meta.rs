use std::path::PathBuf;
use std::sync::Arc;

use persistence::{PersistenceError, Store};

use super::MapMeta;

/// Filesystem-backed store for map metadata.
#[derive(Clone)]
pub struct FsMapMetaStore {
    pub map_dir: Arc<PathBuf>,
}

impl Store<(), MapMeta> for FsMapMetaStore {
    fn save(&self, _key: &(), value: &MapMeta) -> Result<(), PersistenceError> {
        super::save_map_meta(&self.map_dir, value).map_err(PersistenceError::Serialize)
    }

    fn load(&self, _key: &()) -> Result<Option<MapMeta>, PersistenceError> {
        super::load_map_meta(&self.map_dir).map_err(PersistenceError::Deserialize)
    }
}
