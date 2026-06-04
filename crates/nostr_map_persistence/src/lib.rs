pub mod attestation;
pub mod manifest;
pub mod payloads;
pub mod policy;
pub mod publish;
pub mod read;
pub mod stores;
pub mod validation;

pub use attestation::{
    sign_homebase_attestation, verify_homebase_attestation, verify_homebase_attestation_signature,
    AttestationSigner, AttestationVerifier, HOMEBASE_ATTESTATION_SIGNING_DOMAIN,
};
pub use payloads::{
    encode_chunk_entities_payload, encode_chunk_payload, encode_map_entities_payload,
    encode_map_meta_payload, prepare_publish_slot, upload_prepared_slots, PreparedPublishSlot,
    CHUNK_ENTITIES_SCHEMA_VERSION, MAP_ENTITIES_SCHEMA_VERSION, MAP_META_SCHEMA_VERSION,
    MAX_CONCURRENT_BLOB_TRANSFERS,
};
pub use protocol::{HomebasePayloadScope, HomebasePublicationAttestation};
pub use validation::validate_homebase_manifest_attestation;

pub use manifest::{
    compute_descriptor_root, compute_manifest_hash, finalize_manifest, manifest_event_draft,
    manifest_event_tags, manifest_from_json, manifest_hash_hex, manifest_to_json,
    verify_descriptor_root, verify_manifest_event, ManifestHash, ManifestPayloadDescriptor,
    ManifestPayloadSlot, MapPersistenceRejection, MapRevision, NostrMapManifest, PayloadClass,
    PayloadKey, PayloadSlotState, RawChunkEntitiesPayload, RawChunkPayload, RawMapEntitiesPayload,
    RawMapMetaPayload, RawMapPayloads, RawSaveBase, RawValidatedMapDelta, RawValidatedMapSave,
    MANIFEST_HASH_TAG, MAP_MANIFEST_SCHEMA_VERSION, MAP_TAG, NOSTR_KIND_MAP_MANIFEST,
    PREVIOUS_MANIFEST_HASH_TAG,
};
pub use nostr_client::BlobRef;
pub use policy::{MapPersistencePolicy, NostrMapQueryPolicy};
pub use publish::{
    build_signed_map_manifest_event, manifest_hash_from_signed_event_json, MapManifestSigner,
};
pub use read::{
    assemble_raw_validated_map_save, download_payloads, fetch_manifest_ancestors,
    latest_visible_manifest, validate_remote_map_save, verify_revision_chain,
    RemotePersistenceError, RevisionDecision,
};
pub use stores::{BlossomBlobPutStore, NostrManifestPublishStore};
