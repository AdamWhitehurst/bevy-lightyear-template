use std::{path::PathBuf, sync::Arc};

use nostr_client::{
    client_identity_dir, load_encrypted_identity_from_dir, save_encrypted_identity_to_dir,
    EncryptedIdentity, IdentityStoreError,
};
use persistence::{PersistenceError, Store};

#[derive(Clone)]
pub struct FsEncryptedIdentityStore {
    pub base_dir: Arc<PathBuf>,
}

pub fn default_nostr_identity_dir() -> PathBuf {
    client_identity_dir(None).expect("default identity profile is valid")
}

pub fn nostr_identity_dir(profile: Option<&str>) -> Result<PathBuf, String> {
    client_identity_dir(profile)
}

impl Store<(), EncryptedIdentity> for FsEncryptedIdentityStore {
    fn save(&self, _key: &(), value: &EncryptedIdentity) -> Result<(), PersistenceError> {
        save_encrypted_identity_to_dir(self.base_dir.as_ref(), value)
            .map_err(identity_store_save_error_to_persistence)
    }

    fn load(&self, _key: &()) -> Result<Option<EncryptedIdentity>, PersistenceError> {
        load_encrypted_identity_from_dir(self.base_dir.as_ref())
            .map_err(identity_store_load_error_to_persistence)
    }
}

fn identity_store_save_error_to_persistence(error: IdentityStoreError) -> PersistenceError {
    match error {
        IdentityStoreError::Io(message) | IdentityStoreError::Serialize(message) => {
            PersistenceError::Serialize(message)
        }
        IdentityStoreError::Deserialize(message) => PersistenceError::Deserialize(message),
        IdentityStoreError::VersionMismatch { expected, actual } => {
            PersistenceError::VersionMismatch { expected, actual }
        }
    }
}

fn identity_store_load_error_to_persistence(error: IdentityStoreError) -> PersistenceError {
    match error {
        IdentityStoreError::Io(message) | IdentityStoreError::Deserialize(message) => {
            PersistenceError::Deserialize(message)
        }
        IdentityStoreError::Serialize(message) => PersistenceError::Serialize(message),
        IdentityStoreError::VersionMismatch { expected, actual } => {
            PersistenceError::VersionMismatch { expected, actual }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_client::{
        generate_encrypted_identity, identity::ENCRYPTED_IDENTITY_VERSION, EncryptedIdentity,
    };

    fn test_store(dir: &std::path::Path) -> FsEncryptedIdentityStore {
        FsEncryptedIdentityStore {
            base_dir: Arc::new(dir.to_path_buf()),
        }
    }

    #[test]
    fn named_identity_dir_delegates_to_shared_profile_path() {
        assert_eq!(
            nostr_identity_dir(Some("alice_1")).unwrap(),
            client_identity_dir(Some("alice_1")).unwrap()
        );
    }

    #[test]
    fn load_wrong_version_returns_version_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let wrong_version = EncryptedIdentity {
            version: nostr_client::identity::ENCRYPTED_IDENTITY_VERSION + 1,
            ciphertext: "ncryptsec1invalid".to_string(),
        };
        save_encrypted_identity_to_dir(dir.path(), &wrong_version).unwrap();
        let store = test_store(dir.path());

        match store.load(&()).unwrap_err() {
            PersistenceError::VersionMismatch { expected, actual } => {
                assert_eq!(expected, ENCRYPTED_IDENTITY_VERSION);
                assert_eq!(actual, ENCRYPTED_IDENTITY_VERSION + 1);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn load_missing_identity_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());

        assert!(store.load(&()).unwrap().is_none());
    }

    #[test]
    fn save_load_identity_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let (_identity, encrypted) = generate_encrypted_identity("passphrase").unwrap();

        store.save(&(), &encrypted).unwrap();
        let loaded = store.load(&()).unwrap().expect("identity should exist");

        assert_eq!(loaded.version, ENCRYPTED_IDENTITY_VERSION);
        assert_eq!(loaded.ciphertext, encrypted.ciphertext);
    }

    #[test]
    fn save_uses_atomic_tmp_path() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let (_identity, encrypted) = generate_encrypted_identity("passphrase").unwrap();

        store.save(&(), &encrypted).unwrap();

        assert!(dir.path().join("identity.bin").exists());
        assert!(!dir.path().join("identity.bin.tmp").exists());
    }
}
