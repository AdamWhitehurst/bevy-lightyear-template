use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::types::ChunkData;

const CHUNK_SAVE_VERSION: u32 = 3;
const ZSTD_COMPRESSION_LEVEL: i32 = 3;

/// Versioned envelope wrapping chunk data on disk.
#[derive(Serialize, Deserialize)]
struct ChunkFileEnvelope {
    version: u32,
    data: ChunkData,
}

/// Returns the file path for a chunk at the given position within a map directory.
pub fn chunk_file_path(map_dir: &Path, chunk_pos: IVec3) -> PathBuf {
    map_dir.join("terrain").join(format!(
        "chunk_{}_{}_{}.bin",
        chunk_pos.x, chunk_pos.y, chunk_pos.z
    ))
}

/// Save chunk data to a compressed file. Uses atomic write via tmp+rename.
pub fn save_chunk(map_dir: &Path, chunk_pos: IVec3, data: &ChunkData) -> Result<(), String> {
    let path = chunk_file_path(map_dir, chunk_pos);
    fs::create_dir_all(path.parent().expect("chunk path has parent"))
        .map_err(|e| format!("mkdir terrain: {e}"))?;

    let envelope = ChunkFileEnvelope {
        version: CHUNK_SAVE_VERSION,
        data: data.clone(),
    };
    let bytes = bincode::serialize(&envelope).map_err(|e| format!("serialize chunk: {e}"))?;

    let tmp_path = path.with_extension("bin.tmp");
    let file = fs::File::create(&tmp_path).map_err(|e| format!("create tmp: {e}"))?;
    let mut encoder = zstd::Encoder::new(file, ZSTD_COMPRESSION_LEVEL)
        .map_err(|e| format!("zstd encoder: {e}"))?;
    encoder
        .write_all(&bytes)
        .map_err(|e| format!("write chunk: {e}"))?;
    encoder.finish().map_err(|e| format!("zstd finish: {e}"))?;

    fs::rename(&tmp_path, &path).map_err(|e| format!("atomic rename: {e}"))?;
    Ok(())
}

/// Load chunk data from disk. Returns `None` if the file does not exist.
pub fn load_chunk(map_dir: &Path, chunk_pos: IVec3) -> Result<Option<ChunkData>, String> {
    let path = chunk_file_path(map_dir, chunk_pos);
    if !path.exists() {
        return Ok(None);
    }

    let file = fs::File::open(&path).map_err(|e| format!("open chunk: {e}"))?;
    let mut decoder = zstd::Decoder::new(file).map_err(|e| format!("zstd decoder: {e}"))?;
    let mut bytes = Vec::new();
    decoder
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read chunk: {e}"))?;

    let envelope: ChunkFileEnvelope =
        bincode::deserialize(&bytes).map_err(|e| format!("deserialize chunk: {e}"))?;

    if envelope.version != CHUNK_SAVE_VERSION {
        return Err(format!(
            "chunk version mismatch: expected {CHUNK_SAVE_VERSION}, got {}",
            envelope.version
        ));
    }

    Ok(Some(envelope.data))
}

/// Delete a saved chunk file if it exists.
pub fn delete_chunk(map_dir: &Path, chunk_pos: IVec3) -> Result<(), String> {
    let path = chunk_file_path(map_dir, chunk_pos);
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("delete chunk: {e}"))?;
    }
    Ok(())
}

/// List all chunk positions that have saved files in the terrain directory.
pub fn list_saved_chunks(map_dir: &Path) -> Result<Vec<IVec3>, String> {
    let terrain_dir = map_dir.join("terrain");
    if !terrain_dir.exists() {
        return Ok(Vec::new());
    }

    let mut positions = Vec::new();
    for entry in fs::read_dir(&terrain_dir).map_err(|e| format!("read_dir: {e}"))? {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(pos) = parse_chunk_filename(&name) {
            positions.push(pos);
        }
    }
    Ok(positions)
}

/// Parse a chunk filename like `chunk_1_-2_3.bin` into an `IVec3`.
pub fn parse_chunk_filename(name: &str) -> Option<IVec3> {
    let name = name.strip_prefix("chunk_")?.strip_suffix(".bin")?;
    let last_sep = name.rfind('_')?;
    let z: i32 = name[last_sep + 1..].parse().ok()?;
    let rest = &name[..last_sep];
    let mid_sep = rest.rfind('_')?;
    let y: i32 = rest[mid_sep + 1..].parse().ok()?;
    let x: i32 = rest[..mid_sep].parse().ok()?;
    Some(IVec3::new(x, y, z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChunkStatus, PaddedChunkShape, WorldVoxel};
    use ndshape::ConstShape;

    #[test]
    fn save_load_chunk_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let pos = IVec3::new(1, -2, 3);
        let mut voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        voxels[100] = WorldVoxel::Solid(5);
        let chunk = ChunkData::from_voxels(&voxels, ChunkStatus::Full);

        save_chunk(dir.path(), pos, &chunk).unwrap();
        let loaded = load_chunk(dir.path(), pos)
            .unwrap()
            .expect("chunk should exist");
        assert_eq!(loaded.voxels.to_voxels(), chunk.voxels.to_voxels());
    }

    #[test]
    fn load_nonexistent_chunk_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_chunk(dir.path(), IVec3::ZERO).unwrap().is_none());
    }

    #[test]
    fn save_chunk_creates_directories() {
        let dir = tempfile::tempdir().unwrap();
        let map_dir = dir.path().join("deep/nested/map");
        let voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        save_chunk(
            &map_dir,
            IVec3::ZERO,
            &ChunkData::from_voxels(&voxels, ChunkStatus::Full),
        )
        .unwrap();
        assert!(map_dir.join("terrain").exists());
    }

    #[test]
    fn corrupt_chunk_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = chunk_file_path(dir.path(), IVec3::ZERO);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not valid data").unwrap();
        assert!(load_chunk(dir.path(), IVec3::ZERO).is_err());
    }

    #[test]
    fn parse_chunk_filename_valid() {
        assert_eq!(
            parse_chunk_filename("chunk_1_2_3.bin"),
            Some(IVec3::new(1, 2, 3))
        );
        assert_eq!(parse_chunk_filename("chunk_0_0_0.bin"), Some(IVec3::ZERO));
    }

    #[test]
    fn parse_chunk_filename_negative_coords() {
        assert_eq!(
            parse_chunk_filename("chunk_-1_0_2.bin"),
            Some(IVec3::new(-1, 0, 2))
        );
        assert_eq!(
            parse_chunk_filename("chunk_-10_-20_-30.bin"),
            Some(IVec3::new(-10, -20, -30))
        );
    }

    #[test]
    fn parse_chunk_filename_invalid() {
        assert_eq!(parse_chunk_filename("not_a_chunk.bin"), None);
        assert_eq!(parse_chunk_filename("chunk_1_2.bin"), None);
        assert_eq!(parse_chunk_filename("chunk_a_b_c.bin"), None);
        assert_eq!(parse_chunk_filename("chunk_1_2_3.txt"), None);
    }

    #[test]
    fn list_saved_chunks_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list_saved_chunks(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn list_saved_chunks_with_files() {
        let dir = tempfile::tempdir().unwrap();
        let voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        let chunk = ChunkData::from_voxels(&voxels, ChunkStatus::Full);
        let positions = [IVec3::new(0, 0, 0), IVec3::new(1, -1, 2)];
        for &pos in &positions {
            save_chunk(dir.path(), pos, &chunk).unwrap();
        }
        let mut found = list_saved_chunks(dir.path()).unwrap();
        found.sort_by_key(|p| (p.x, p.y, p.z));
        let mut expected = positions.to_vec();
        expected.sort_by_key(|p| (p.x, p.y, p.z));
        assert_eq!(found, expected);
    }

    #[test]
    fn delete_chunk_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        save_chunk(
            dir.path(),
            IVec3::ZERO,
            &ChunkData::from_voxels(&voxels, ChunkStatus::Full),
        )
        .unwrap();
        assert!(chunk_file_path(dir.path(), IVec3::ZERO).exists());
        delete_chunk(dir.path(), IVec3::ZERO).unwrap();
        assert!(!chunk_file_path(dir.path(), IVec3::ZERO).exists());
    }

    #[test]
    fn delete_nonexistent_chunk_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        delete_chunk(dir.path(), IVec3::ZERO).unwrap();
    }

    #[test]
    fn chunk_data_zstd_compression_reduces_size() {
        let dir = tempfile::tempdir().unwrap();
        // Use a mixed chunk so the palettized representation is large enough
        // for compression to actually reduce size (uniform chunks serialize
        // to only ~24 bytes, smaller than zstd framing overhead).
        let mut voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        for i in 0..100 {
            voxels[i] = WorldVoxel::Solid((i % 5) as u8);
        }
        let chunk = ChunkData::from_voxels(&voxels, ChunkStatus::Full);
        save_chunk(dir.path(), IVec3::ZERO, &chunk).unwrap();

        let path = chunk_file_path(dir.path(), IVec3::ZERO);
        let compressed_size = fs::metadata(&path).unwrap().len();
        let raw_size = bincode::serialize(&ChunkFileEnvelope {
            version: CHUNK_SAVE_VERSION,
            data: chunk,
        })
        .unwrap()
        .len() as u64;

        assert!(
            compressed_size < raw_size / 2,
            "compressed {compressed_size} should be < half of raw {raw_size}"
        );
    }
}
