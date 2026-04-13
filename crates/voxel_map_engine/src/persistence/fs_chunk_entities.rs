use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use persistence::{PersistenceError, Store};

use crate::config::WorldObjectSpawn;

/// Filesystem-backed store for per-chunk entity spawn data.
///
/// Key is `IVec3` (chunk position). Each map entity gets its own store
/// instance with a resolved `map_dir`.
#[derive(Clone)]
pub struct FsChunkEntitiesStore {
    pub map_dir: Arc<PathBuf>,
}

impl Store<IVec3, Vec<WorldObjectSpawn>> for FsChunkEntitiesStore {
    fn save(&self, key: &IVec3, value: &Vec<WorldObjectSpawn>) -> Result<(), PersistenceError> {
        super::save_chunk_entities(&self.map_dir, *key, value).map_err(PersistenceError::Serialize)
    }

    fn load(&self, key: &IVec3) -> Result<Option<Vec<WorldObjectSpawn>>, PersistenceError> {
        super::load_chunk_entities(&self.map_dir, *key).map_err(PersistenceError::Deserialize)
    }
}
