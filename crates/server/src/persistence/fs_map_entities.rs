use std::path::PathBuf;
use std::sync::Arc;

use persistence::{PersistenceError, Store};
use protocol::map::SavedEntity;

/// Filesystem-backed store for map-level entities (respawn points, etc.).
#[derive(Clone)]
pub struct FsMapEntitiesStore {
    pub map_dir: Arc<PathBuf>,
}

impl Store<(), Vec<SavedEntity>> for FsMapEntitiesStore {
    fn save(&self, _key: &(), value: &Vec<SavedEntity>) -> Result<(), PersistenceError> {
        super::save_entities(&self.map_dir, value).map_err(PersistenceError::Serialize)
    }

    fn load(&self, _key: &()) -> Result<Option<Vec<SavedEntity>>, PersistenceError> {
        let entities =
            super::load_entities(&self.map_dir).map_err(PersistenceError::Deserialize)?;
        if entities.is_empty() {
            Ok(None)
        } else {
            Ok(Some(entities))
        }
    }
}
