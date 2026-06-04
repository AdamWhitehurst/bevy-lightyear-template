use futures::stream::{StreamExt, TryStreamExt};
use nostr_client::BlobRef;
use persistence::AsyncStore;
use protocol::map::SavedEntity;
use serde::Serialize;
use sha2::{Digest, Sha256};
use voxel_map_engine::config::WorldObjectSpawn;
use voxel_map_engine::persistence::ChunkFileEnvelope;

use crate::manifest::{
    ManifestPayloadDescriptor, ManifestPayloadSlot, MapPersistenceRejection, PayloadClass,
    PayloadKey, PayloadSlotState,
};

/// Schema version for `PayloadClass::MapMeta` payloads.
pub const MAP_META_SCHEMA_VERSION: u32 = 1;
/// Schema version for `PayloadClass::ChunkEntities` payloads.
pub const CHUNK_ENTITIES_SCHEMA_VERSION: u32 = 3;
/// Schema version for `PayloadClass::MapEntities` payloads.
pub const MAP_ENTITIES_SCHEMA_VERSION: u32 = 1;

/// Wire form of map metadata, encoded identically to the server `MapMeta` struct.
///
/// Uses `[f32; 3]` spawn points (not a Bevy `Vec3`) so this crate stays free of a
/// direct Bevy dependency; bincode encodes both identically.
#[derive(Serialize)]
struct MapMetaWire {
    version: u32,
    seed: u64,
    generation_version: u32,
    spawn_points: Vec<[f32; 3]>,
}

/// Encodes map metadata in the same byte format as the filesystem map metadata store.
///
/// Shared so the server save path and the client homebase publish path produce
/// identical bytes. Spawn points are passed as `[f32; 3]` triples.
pub fn encode_map_meta_payload(
    version: u32,
    seed: u64,
    generation_version: u32,
    spawn_points: Vec<[f32; 3]>,
) -> Result<Vec<u8>, MapPersistenceRejection> {
    bincode::serialize(&MapMetaWire {
        version,
        seed,
        generation_version,
        spawn_points,
    })
    .map_err(|error| MapPersistenceRejection::Invalid(format!("encode map meta: {error}")))
}

/// Encodes terrain chunk data in the same format as the filesystem terrain store.
pub fn encode_chunk_payload(value: ChunkFileEnvelope) -> Result<Vec<u8>, MapPersistenceRejection> {
    zstd_bincode_encode(&value, "chunk payload")
}

/// Encodes chunk entity data in the same format as the filesystem chunk entity store.
pub fn encode_chunk_entities_payload(
    value: Vec<WorldObjectSpawn>,
) -> Result<Vec<u8>, MapPersistenceRejection> {
    #[derive(Serialize)]
    struct Envelope {
        version: u32,
        spawns: Vec<WorldObjectSpawn>,
    }
    zstd_bincode_encode(
        &Envelope {
            version: CHUNK_ENTITIES_SCHEMA_VERSION,
            spawns: value,
        },
        "chunk entities payload",
    )
}

/// Encodes map-level entities in the same format as the filesystem entity store.
pub fn encode_map_entities_payload(
    value: Vec<SavedEntity>,
) -> Result<Vec<u8>, MapPersistenceRejection> {
    #[derive(Serialize)]
    struct Envelope {
        version: u32,
        entities: Vec<SavedEntity>,
    }
    bincode::serialize(&Envelope {
        version: MAP_ENTITIES_SCHEMA_VERSION,
        entities: value,
    })
    .map_err(|error| {
        MapPersistenceRejection::Invalid(format!("encode map entities payload: {error}"))
    })
}

fn zstd_bincode_encode<T: Serialize>(
    value: &T,
    label: &str,
) -> Result<Vec<u8>, MapPersistenceRejection> {
    let encoded = bincode::serialize(value)
        .map_err(|error| MapPersistenceRejection::Invalid(format!("encode {label}: {error}")))?;
    zstd::encode_all(encoded.as_slice(), 0)
        .map_err(|error| MapPersistenceRejection::Invalid(format!("compress {label}: {error}")))
}

/// Maximum simultaneous blob transfers per publish or restore operation.
pub const MAX_CONCURRENT_BLOB_TRANSFERS: usize = 8;

/// An encoded manifest payload slot awaiting blob upload.
///
/// Carries the manifest descriptor plus the encoded blob bytes for `Present` slots.
/// Constructors enforce that bytes are attached exactly when the slot is `Present`.
pub struct PreparedPublishSlot {
    descriptor: ManifestPayloadDescriptor,
    bytes: Option<Vec<u8>>,
}

impl PreparedPublishSlot {
    /// Wraps a descriptor that carries no blob bytes (`Empty`/`Absent`/`Tombstoned`).
    pub fn from_descriptor(descriptor: ManifestPayloadDescriptor) -> Self {
        debug_assert!(
            !matches!(descriptor.slot, ManifestPayloadSlot::Present { .. }),
            "present payload slots must carry bytes; use from_present_descriptor"
        );
        Self {
            descriptor,
            bytes: None,
        }
    }

    /// Wraps an already-encoded `Present` descriptor and points its blob URL at the
    /// public Blossom base URL.
    pub fn from_present_descriptor(
        mut descriptor: ManifestPayloadDescriptor,
        bytes: Vec<u8>,
        public_blossom_base_url: &url::Url,
    ) -> Result<Self, MapPersistenceRejection> {
        let ManifestPayloadSlot::Present { blob } = &mut descriptor.slot else {
            return Err(MapPersistenceRejection::Invalid(format!(
                "blob bytes supplied for non-present payload slot {:?} {:?}",
                descriptor.class, descriptor.key
            )));
        };
        blob.urls = vec![blob_get_url(public_blossom_base_url, blob.sha256)];
        Ok(Self {
            descriptor,
            bytes: Some(bytes),
        })
    }
}

/// Builds the public content-addressed GET URL for an uploaded blob.
fn blob_get_url(public_blossom_base_url: &url::Url, sha256: [u8; 32]) -> String {
    let mut get_url = public_blossom_base_url.clone();
    get_url.set_path(&hex::encode(sha256));
    get_url.to_string()
}

/// Encodes one payload slot into a content-addressed publish descriptor.
///
/// Shared by the server overworld publish path and the client homebase publish path.
pub fn prepare_publish_slot<T>(
    public_blossom_base_url: &url::Url,
    class: PayloadClass,
    key: PayloadKey,
    schema_version: u32,
    slot: PayloadSlotState<T>,
    encode: impl FnOnce(T) -> Result<Vec<u8>, MapPersistenceRejection>,
) -> Result<PreparedPublishSlot, MapPersistenceRejection> {
    let (manifest_slot, bytes) = match slot {
        PayloadSlotState::Present(value) => {
            let bytes = encode(value)?;
            let sha256: [u8; 32] = Sha256::digest(&bytes).into();
            let blob = BlobRef {
                sha256,
                size: bytes.len() as u64,
                content_type: "application/octet-stream".to_string(),
                urls: vec![blob_get_url(public_blossom_base_url, sha256)],
            };
            (ManifestPayloadSlot::Present { blob }, Some(bytes))
        }
        PayloadSlotState::Empty => (ManifestPayloadSlot::Empty, None),
        PayloadSlotState::Absent => (ManifestPayloadSlot::Absent, None),
        PayloadSlotState::Tombstoned => (ManifestPayloadSlot::Tombstoned, None),
    };
    Ok(PreparedPublishSlot {
        descriptor: ManifestPayloadDescriptor {
            class,
            key,
            slot: manifest_slot,
            schema_version,
        },
        bytes,
    })
}

/// Uploads all `Present` blobs concurrently (bounded) and returns the manifest descriptors.
///
/// Blobs are content-addressed and independent, so upload completion order does not
/// affect the manifest: `finalize_manifest` canonically sorts descriptors afterwards.
pub async fn upload_prepared_slots(
    blob_store: &impl AsyncStore<BlobRef, Vec<u8>>,
    slots: Vec<PreparedPublishSlot>,
) -> Result<Vec<ManifestPayloadDescriptor>, MapPersistenceRejection> {
    futures::stream::iter(slots)
        .map(|PreparedPublishSlot { descriptor, bytes }| async move {
            if let Some(bytes) = bytes {
                let ManifestPayloadSlot::Present { blob } = &descriptor.slot else {
                    unreachable!("constructors only attach bytes to present slots");
                };
                blob_store
                    .save(blob, &bytes)
                    .await
                    .map_err(|error| MapPersistenceRejection::Unavailable(error.to_string()))?;
            }
            Ok(descriptor)
        })
        .buffered(MAX_CONCURRENT_BLOB_TRANSFERS)
        .try_collect()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use persistence::{BoxedStoreFuture, PersistenceError};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// Blob store double that records uploads and tracks in-flight concurrency.
    #[derive(Clone, Default)]
    struct RecordingBlobStore {
        uploads: Arc<Mutex<Vec<BlobRef>>>,
        in_flight: Arc<AtomicUsize>,
        max_in_flight: Arc<AtomicUsize>,
        fail: bool,
    }

    impl AsyncStore<BlobRef, Vec<u8>> for RecordingBlobStore {
        fn load<'a>(
            &'a self,
            _key: &'a BlobRef,
        ) -> BoxedStoreFuture<'a, Result<Option<Vec<u8>>, PersistenceError>> {
            Box::pin(async { Ok(None) })
        }

        fn save<'a>(
            &'a self,
            key: &'a BlobRef,
            value: &'a Vec<u8>,
        ) -> BoxedStoreFuture<'a, Result<(), PersistenceError>> {
            Box::pin(async move {
                if self.fail {
                    return Err(PersistenceError::Serialize(
                        "forced upload failure".to_string(),
                    ));
                }
                let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_in_flight.fetch_max(now, Ordering::SeqCst);
                // Yield so other queued uploads start before this one completes,
                // making the max_in_flight measurement meaningful.
                futures_lite::future::yield_now().await;
                self.in_flight.fetch_sub(1, Ordering::SeqCst);
                let expected: [u8; 32] = Sha256::digest(value).into();
                assert_eq!(expected, key.sha256, "uploaded bytes must match blob ref");
                self.uploads.lock().unwrap().push(key.clone());
                Ok(())
            })
        }
    }

    fn base_url() -> url::Url {
        url::Url::parse("https://blossom.test/").unwrap()
    }

    fn present_chunk_slot(x: i32, bytes: Vec<u8>) -> PreparedPublishSlot {
        prepare_publish_slot(
            &base_url(),
            PayloadClass::TerrainChunk,
            PayloadKey::Chunk { x, y: 0, z: 0 },
            1,
            PayloadSlotState::Present(bytes),
            Ok,
        )
        .expect("prepare present chunk slot")
    }

    #[test]
    fn payloads_prepare_publish_slot_content_addresses_present_bytes() {
        let bytes = b"chunk-bytes".to_vec();
        let prepared = present_chunk_slot(0, bytes.clone());
        let ManifestPayloadSlot::Present { blob } = &prepared.descriptor.slot else {
            panic!("present slot expected");
        };
        let expected: [u8; 32] = Sha256::digest(&bytes).into();
        assert_eq!(blob.sha256, expected);
        assert_eq!(blob.size, bytes.len() as u64);
        assert_eq!(
            blob.urls,
            vec![format!("https://blossom.test/{}", hex::encode(expected))]
        );
    }

    #[test]
    fn payloads_upload_prepared_slots_uploads_present_blobs_concurrently() {
        let store = RecordingBlobStore::default();
        let tombstone = prepare_publish_slot::<Vec<u8>>(
            &base_url(),
            PayloadClass::TerrainChunk,
            PayloadKey::Chunk { x: 9, y: 0, z: 0 },
            1,
            PayloadSlotState::Tombstoned,
            Ok,
        )
        .expect("prepare tombstoned slot");
        let prepared = vec![
            present_chunk_slot(0, b"zero".to_vec()),
            present_chunk_slot(1, b"one".to_vec()),
            present_chunk_slot(2, b"two".to_vec()),
            tombstone,
        ];

        let descriptors = futures_lite::future::block_on(upload_prepared_slots(&store, prepared))
            .expect("uploads succeed");

        assert_eq!(descriptors.len(), 4);
        assert!(matches!(
            descriptors[3].slot,
            ManifestPayloadSlot::Tombstoned
        ));
        assert_eq!(store.uploads.lock().unwrap().len(), 3);
        assert!(
            store.max_in_flight.load(Ordering::SeqCst) >= 2,
            "present blob uploads must overlap instead of running serially"
        );
    }

    #[test]
    fn payloads_upload_prepared_slots_propagates_upload_failure() {
        let store = RecordingBlobStore {
            fail: true,
            ..Default::default()
        };
        let prepared = vec![present_chunk_slot(0, b"zero".to_vec())];
        let error = futures_lite::future::block_on(upload_prepared_slots(&store, prepared))
            .expect_err("upload failure propagates");
        assert!(matches!(error, MapPersistenceRejection::Unavailable(_)));
    }
}
