use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use persistence::{PersistenceError, Store};

use super::{MapMeta, META_VERSION};

/// Filesystem-backed store for map metadata.
#[derive(Clone)]
pub struct FsMapMetaStore {
    pub map_dir: Arc<PathBuf>,
}

impl Store<(), MapMeta> for FsMapMetaStore {
    fn save(&self, _key: &(), value: &MapMeta) -> Result<(), PersistenceError> {
        fs::create_dir_all(self.map_dir.as_ref())
            .map_err(|e| PersistenceError::Serialize(format!("mkdir map_dir: {e}")))?;
        let path = self.map_dir.join("map.meta.bin");
        let bytes = bincode::serialize(value)
            .map_err(|e| PersistenceError::Serialize(format!("serialize meta: {e}")))?;
        let tmp_path = path.with_extension("bin.tmp");
        fs::write(&tmp_path, &bytes)
            .map_err(|e| PersistenceError::Serialize(format!("write meta tmp: {e}")))?;
        fs::rename(&tmp_path, &path)
            .map_err(|e| PersistenceError::Serialize(format!("rename meta: {e}")))?;
        Ok(())
    }

    fn load(&self, _key: &()) -> Result<Option<MapMeta>, PersistenceError> {
        let path = self.map_dir.join("map.meta.bin");
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path)
            .map_err(|e| PersistenceError::Deserialize(format!("read meta: {e}")))?;
        let meta: MapMeta = bincode::deserialize(&bytes)
            .map_err(|e| PersistenceError::Deserialize(format!("deserialize meta: {e}")))?;
        if meta.version != META_VERSION {
            return Err(PersistenceError::VersionMismatch {
                expected: META_VERSION,
                actual: meta.version,
            });
        }
        Ok(Some(meta))
    }
}
