use nostr_client::BlobRef;
use persistence::{AsyncStore, BoxedStoreFuture, PersistenceError};

use crate::manifest::ManifestHash;
use crate::publish::manifest_hash_from_signed_event_json;

/// Async write-only store for Blossom/content-addressed blob uploads.
#[derive(Clone)]
pub struct BlossomBlobPutStore {
    pub upload_url: String,
    pub auth: nostr_client::BlossomAuth,
}

impl AsyncStore<BlobRef, Vec<u8>> for BlossomBlobPutStore {
    fn load<'a>(
        &'a self,
        _key: &'a BlobRef,
    ) -> BoxedStoreFuture<'a, Result<Option<Vec<u8>>, PersistenceError>> {
        Box::pin(async {
            Err(PersistenceError::Deserialize(
                "BlossomBlobPutStore does not support load".to_string(),
            ))
        })
    }

    fn save<'a>(
        &'a self,
        key: &'a BlobRef,
        value: &'a Vec<u8>,
    ) -> BoxedStoreFuture<'a, Result<(), PersistenceError>> {
        Box::pin(async move {
            nostr_client::blobs::verify_blob_bytes(key.sha256, Some(key.size), value.clone())
                .map_err(|error| {
                    PersistenceError::Serialize(format!("verify upload blob bytes: {error}"))
                })?;
            let uploaded =
                nostr_client::blobs::upload_blob(&self.upload_url, value.clone(), &self.auth)
                    .await
                    .map_err(|error| {
                        PersistenceError::Serialize(format!("upload Blossom blob: {error}"))
                    })?;
            if uploaded.sha256 != key.sha256 || uploaded.size != key.size {
                return Err(PersistenceError::Serialize(format!(
                    "uploaded blob ref mismatch: expected {} bytes {:?}, got {} bytes {:?}",
                    key.size, key.sha256, uploaded.size, uploaded.sha256
                )));
            }
            Ok(())
        })
    }
}

/// Async write-only store for publishing signed Nostr manifest events.
#[derive(Clone)]
pub struct NostrManifestPublishStore {
    pub client: nostr_client::events::NostrEventClient,
}

impl AsyncStore<ManifestHash, String> for NostrManifestPublishStore {
    fn load<'a>(
        &'a self,
        _key: &'a ManifestHash,
    ) -> BoxedStoreFuture<'a, Result<Option<String>, PersistenceError>> {
        Box::pin(async {
            Err(PersistenceError::Deserialize(
                "NostrManifestPublishStore does not support load".to_string(),
            ))
        })
    }

    fn save<'a>(
        &'a self,
        key: &'a ManifestHash,
        value: &'a String,
    ) -> BoxedStoreFuture<'a, Result<(), PersistenceError>> {
        Box::pin(async move {
            let actual_hash = manifest_hash_from_signed_event_json(value).map_err(|error| {
                PersistenceError::Serialize(format!("verify manifest publish event: {error}"))
            })?;
            if actual_hash != *key {
                return Err(PersistenceError::Serialize(format!(
                    "manifest publish hash mismatch: expected {:?}, got {:?}",
                    key, actual_hash
                )));
            }
            nostr_client::events::publish_event(&self.client, value.clone())
                .await
                .map_err(|error| {
                    PersistenceError::Serialize(format!("publish Nostr manifest event: {error}"))
                })
        })
    }
}
