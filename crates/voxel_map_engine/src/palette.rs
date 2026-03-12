use bevy::reflect::Reflect;
use serde::{Deserialize, Serialize};

use crate::types::WorldVoxel;

const PADDED_VOLUME: usize = 18 * 18 * 18;

/// Minimum bits to represent `count` distinct values.
///
/// Returns 0 for count <= 1 (single-value case needs no index bits).
fn bits_needed(count: usize) -> u8 {
    match count {
        0 | 1 => 0,
        2 => 1,
        n => (usize::BITS - (n - 1).leading_zeros()) as u8,
    }
}

/// Palette-based chunk storage with two strategies.
///
/// Compresses a fixed-size voxel array (18^3 = 5832 entries) by deduplicating
/// voxel types into a palette and storing small indices instead of full values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum PalettedChunk {
    /// All voxels are the same value. Near-zero overhead.
    SingleValue(WorldVoxel),

    /// 2-256 distinct voxel types. Palette array + packed bit indices.
    Indirect {
        /// Distinct voxel types in this chunk.
        palette: Vec<WorldVoxel>,
        /// Packed indices into `palette`. Each index is `bits_per_entry` wide,
        /// stored left-to-right within each u64. Indices never span u64 boundaries.
        data: Vec<u64>,
        /// Bits per palette index.
        bits_per_entry: u8,
        /// Number of logical entries (voxel count).
        len: usize,
    },
}

impl PalettedChunk {
    /// Build a `PalettedChunk` from a flat voxel slice.
    ///
    /// Uses `SingleValue` when all voxels are identical, `Indirect` otherwise.
    pub fn from_voxels(voxels: &[WorldVoxel]) -> Self {
        assert_eq!(
            voxels.len(),
            PADDED_VOLUME,
            "expected {PADDED_VOLUME} voxels"
        );

        let palette = build_palette(voxels);
        if palette.len() <= 1 {
            return Self::SingleValue(voxels[0]);
        }

        let bits = bits_needed(palette.len());
        let data = pack_indices(voxels, &palette, bits);

        Self::Indirect {
            palette,
            data,
            bits_per_entry: bits,
            len: voxels.len(),
        }
    }

    /// Expand back to a flat voxel array.
    pub fn to_voxels(&self) -> Vec<WorldVoxel> {
        match self {
            Self::SingleValue(v) => vec![*v; PADDED_VOLUME],
            Self::Indirect {
                palette,
                data,
                bits_per_entry,
                len,
            } => {
                let mut out = Vec::with_capacity(*len);
                let entries_per_word = 64 / (*bits_per_entry as usize);
                let mask = (1u64 << *bits_per_entry) - 1;

                for i in 0..*len {
                    let word_idx = i / entries_per_word;
                    let bit_offset = (i % entries_per_word) * (*bits_per_entry as usize);
                    let index = ((data[word_idx] >> bit_offset) & mask) as usize;
                    out.push(palette[index]);
                }
                out
            }
        }
    }

    /// O(1) indexed voxel access.
    pub fn get(&self, index: usize) -> WorldVoxel {
        match self {
            Self::SingleValue(v) => {
                debug_assert!(index < PADDED_VOLUME, "index {index} out of bounds");
                *v
            }
            Self::Indirect {
                palette,
                data,
                bits_per_entry,
                len,
            } => {
                debug_assert!(index < *len, "index {index} out of bounds (len {len})");
                let entries_per_word = 64 / (*bits_per_entry as usize);
                let mask = (1u64 << *bits_per_entry) - 1;
                let word_idx = index / entries_per_word;
                let bit_offset = (index % entries_per_word) * (*bits_per_entry as usize);
                let palette_idx = ((data[word_idx] >> bit_offset) & mask) as usize;
                palette[palette_idx]
            }
        }
    }

    /// Mutate a single voxel.
    ///
    /// If the voxel type is already in the palette, updates in-place.
    /// If it's a new type, rebuilds the chunk. No-op when setting a `SingleValue`
    /// to its existing value.
    pub fn set(&mut self, index: usize, voxel: WorldVoxel) {
        match self {
            Self::SingleValue(v) => {
                if *v == voxel {
                    return;
                }
                let mut expanded = vec![*v; PADDED_VOLUME];
                expanded[index] = voxel;
                *self = Self::from_voxels(&expanded);
            }
            Self::Indirect {
                palette,
                data,
                bits_per_entry,
                len: _,
            } => {
                if let Some(palette_idx) = palette.iter().position(|&p| p == voxel) {
                    let entries_per_word = 64 / (*bits_per_entry as usize);
                    let mask = (1u64 << *bits_per_entry) - 1;
                    let word_idx = index / entries_per_word;
                    let bit_offset = (index % entries_per_word) * (*bits_per_entry as usize);
                    data[word_idx] &= !(mask << bit_offset);
                    data[word_idx] |= (palette_idx as u64) << bit_offset;
                } else {
                    let mut expanded = self.to_voxels();
                    expanded[index] = voxel;
                    *self = Self::from_voxels(&expanded);
                }
            }
        }
    }

    /// True if every voxel in the chunk is the same value.
    pub fn is_uniform(&self) -> bool {
        matches!(self, Self::SingleValue(_))
    }

    /// Number of distinct voxel types in the palette.
    pub fn palette_size(&self) -> usize {
        match self {
            Self::SingleValue(_) => 1,
            Self::Indirect { palette, .. } => palette.len(),
        }
    }

    /// Approximate heap memory usage in bytes (excludes the enum itself).
    pub fn memory_usage(&self) -> usize {
        match self {
            Self::SingleValue(_) => 0,
            Self::Indirect {
                palette,
                data,
                bits_per_entry: _,
                len: _,
            } => palette.len() * size_of::<WorldVoxel>() + data.len() * size_of::<u64>(),
        }
    }
}

/// Build a deduplicated palette preserving insertion order.
fn build_palette(voxels: &[WorldVoxel]) -> Vec<WorldVoxel> {
    let mut palette = Vec::new();
    for &v in voxels {
        if !palette.contains(&v) {
            palette.push(v);
        }
    }
    palette
}

/// Pack voxel indices into u64 words. Indices never span word boundaries.
fn pack_indices(voxels: &[WorldVoxel], palette: &[WorldVoxel], bits: u8) -> Vec<u64> {
    let entries_per_word = 64 / (bits as usize);
    let num_words = (voxels.len() + entries_per_word - 1) / entries_per_word;
    let mut data = vec![0u64; num_words];

    for (i, &voxel) in voxels.iter().enumerate() {
        let palette_idx = palette
            .iter()
            .position(|&p| p == voxel)
            .expect("voxel must exist in palette");
        let word_idx = i / entries_per_word;
        let bit_offset = (i % entries_per_word) * (bits as usize);
        data[word_idx] |= (palette_idx as u64) << bit_offset;
    }

    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_needed_values() {
        assert_eq!(bits_needed(1), 0);
        assert_eq!(bits_needed(2), 1);
        assert_eq!(bits_needed(3), 2);
        assert_eq!(bits_needed(4), 2);
        assert_eq!(bits_needed(5), 3);
        assert_eq!(bits_needed(8), 3);
        assert_eq!(bits_needed(9), 4);
        assert_eq!(bits_needed(256), 8);
    }

    #[test]
    fn single_value_air() {
        let voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        let chunk = PalettedChunk::from_voxels(&voxels);
        assert!(chunk.is_uniform());
        assert_eq!(chunk.palette_size(), 1);
        assert_eq!(chunk.get(0), WorldVoxel::Air);
        assert_eq!(chunk.get(PADDED_VOLUME - 1), WorldVoxel::Air);
        assert_eq!(chunk.to_voxels(), voxels);
    }

    #[test]
    fn single_value_solid() {
        let voxels = vec![WorldVoxel::Solid(7); PADDED_VOLUME];
        let chunk = PalettedChunk::from_voxels(&voxels);
        assert!(chunk.is_uniform());
        assert_eq!(chunk.get(100), WorldVoxel::Solid(7));
    }

    #[test]
    fn two_voxel_types_roundtrip() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        for i in (0..PADDED_VOLUME).step_by(2) {
            voxels[i] = WorldVoxel::Solid(1);
        }
        let chunk = PalettedChunk::from_voxels(&voxels);
        assert!(!chunk.is_uniform());
        assert_eq!(chunk.palette_size(), 2);
        assert_eq!(chunk.to_voxels(), voxels);
    }

    #[test]
    fn many_voxel_types_roundtrip() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        for i in 0..200 {
            voxels[i] = WorldVoxel::Solid(i as u8);
        }
        let chunk = PalettedChunk::from_voxels(&voxels);
        assert_eq!(chunk.palette_size(), 201); // Air + 200 solid variants (Solid(0) distinct from Air)
        assert_eq!(chunk.to_voxels(), voxels);
    }

    #[test]
    fn get_single_voxel_indexed_access() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        voxels[42] = WorldVoxel::Solid(99);
        let chunk = PalettedChunk::from_voxels(&voxels);
        assert_eq!(chunk.get(0), WorldVoxel::Air);
        assert_eq!(chunk.get(42), WorldVoxel::Solid(99));
        assert_eq!(chunk.get(43), WorldVoxel::Air);
    }

    #[test]
    fn set_within_existing_palette() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        voxels[0] = WorldVoxel::Solid(1);
        let mut chunk = PalettedChunk::from_voxels(&voxels);
        let old_palette_size = chunk.palette_size();

        chunk.set(1, WorldVoxel::Solid(1));

        assert_eq!(chunk.get(1), WorldVoxel::Solid(1));
        assert_eq!(chunk.palette_size(), old_palette_size);
    }

    #[test]
    fn set_expands_palette() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        voxels[0] = WorldVoxel::Solid(1);
        let mut chunk = PalettedChunk::from_voxels(&voxels);
        assert_eq!(chunk.palette_size(), 2);

        chunk.set(1, WorldVoxel::Solid(2));

        assert_eq!(chunk.get(1), WorldVoxel::Solid(2));
        assert_eq!(chunk.palette_size(), 3);
    }

    #[test]
    fn set_transitions_from_single_value() {
        let mut chunk = PalettedChunk::SingleValue(WorldVoxel::Air);
        chunk.set(0, WorldVoxel::Solid(5));
        assert!(!chunk.is_uniform());
        assert_eq!(chunk.get(0), WorldVoxel::Solid(5));
        assert_eq!(chunk.get(1), WorldVoxel::Air);
    }

    #[test]
    fn set_noop_on_single_value() {
        let mut chunk = PalettedChunk::SingleValue(WorldVoxel::Air);
        chunk.set(0, WorldVoxel::Air);
        assert!(chunk.is_uniform());
    }

    #[test]
    fn memory_usage_single_value_minimal() {
        let chunk = PalettedChunk::SingleValue(WorldVoxel::Air);
        assert!(chunk.memory_usage() < 16);
    }

    #[test]
    fn memory_usage_indirect_less_than_flat() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        voxels[0] = WorldVoxel::Solid(1);
        let chunk = PalettedChunk::from_voxels(&voxels);
        let flat_size = PADDED_VOLUME * size_of::<WorldVoxel>();
        assert!(
            chunk.memory_usage() < flat_size / 4,
            "indirect {} should be < flat/4 {}",
            chunk.memory_usage(),
            flat_size / 4,
        );
    }

    #[test]
    fn serde_roundtrip() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        for i in 0..50 {
            voxels[i * 10] = WorldVoxel::Solid(i as u8);
        }
        let chunk = PalettedChunk::from_voxels(&voxels);
        let bytes = bincode::serialize(&chunk).expect("serialize");
        let restored: PalettedChunk = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(restored.to_voxels(), voxels);
    }

    #[test]
    fn serde_single_value_roundtrip() {
        let chunk = PalettedChunk::SingleValue(WorldVoxel::Solid(3));
        let bytes = bincode::serialize(&chunk).expect("serialize");
        let restored: PalettedChunk = bincode::deserialize(&bytes).expect("deserialize");
        assert!(restored.is_uniform());
        assert_eq!(restored.get(0), WorldVoxel::Solid(3));
    }
}
