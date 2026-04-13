use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use persistence::{PersistenceError, Store};

use super::{CHUNK_SAVE_VERSION, ChunkFileEnvelope};

/// Filesystem-backed store for chunk terrain data.
///
/// Key is `IVec3` (chunk position). Each map entity gets its own store
/// instance with a resolved `map_dir`.
#[derive(Clone)]
pub struct FsChunkStore {
    pub map_dir: Arc<PathBuf>,
}

impl Store<IVec3, ChunkFileEnvelope> for FsChunkStore {
    fn save(&self, key: &IVec3, value: &ChunkFileEnvelope) -> Result<(), PersistenceError> {
        super::save_chunk(&self.map_dir, *key, value.chunk_size, &value.data)
            .map_err(PersistenceError::Serialize)
    }

    fn load(&self, key: &IVec3) -> Result<Option<ChunkFileEnvelope>, PersistenceError> {
        let path = super::chunk_file_path(&self.map_dir, *key);
        if !path.exists() {
            return Ok(None);
        }

        let file = std::fs::File::open(&path)
            .map_err(|e| PersistenceError::Deserialize(format!("open chunk: {e}")))?;
        let mut decoder = zstd::Decoder::new(file)
            .map_err(|e| PersistenceError::Deserialize(format!("zstd decoder: {e}")))?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut bytes)
            .map_err(|e| PersistenceError::Deserialize(format!("read chunk: {e}")))?;

        let envelope: ChunkFileEnvelope = bincode::deserialize(&bytes)
            .map_err(|e| PersistenceError::Deserialize(format!("deserialize chunk: {e}")))?;

        if envelope.version != CHUNK_SAVE_VERSION {
            return Err(PersistenceError::VersionMismatch {
                expected: CHUNK_SAVE_VERSION,
                actual: envelope.version,
            });
        }

        Ok(Some(envelope))
    }
}
