use std::{fs, path::PathBuf, sync::Arc};

use nostr_client::{client_identity_dir, identity::ENCRYPTED_IDENTITY_VERSION, EncryptedIdentity};
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
        fs::create_dir_all(self.base_dir.as_ref())
            .map_err(|error| PersistenceError::Serialize(format!("mkdir identity dir: {error}")))?;
        let path = self.base_dir.join("identity.bin");
        let bytes = bincode::serialize(value)
            .map_err(|error| PersistenceError::Serialize(format!("serialize identity: {error}")))?;
        let tmp_path = path.with_extension("bin.tmp");
        fs::write(&tmp_path, &bytes)
            .map_err(|error| PersistenceError::Serialize(format!("write identity tmp: {error}")))?;
        fs::rename(&tmp_path, &path)
            .map_err(|error| PersistenceError::Serialize(format!("rename identity: {error}")))?;
        Ok(())
    }

    fn load(&self, _key: &()) -> Result<Option<EncryptedIdentity>, PersistenceError> {
        let path = self.base_dir.join("identity.bin");
        if !path.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&path)
            .map_err(|error| PersistenceError::Deserialize(format!("read identity: {error}")))?;
        let identity: EncryptedIdentity = bincode::deserialize(&bytes).map_err(|error| {
            PersistenceError::Deserialize(format!("deserialize identity: {error}"))
        })?;
        if identity.version != ENCRYPTED_IDENTITY_VERSION {
            return Err(PersistenceError::VersionMismatch {
                expected: ENCRYPTED_IDENTITY_VERSION,
                actual: identity.version,
            });
        }

        Ok(Some(identity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_client::{generate_encrypted_identity, EncryptedIdentity};

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
    fn wrong_version_returns_version_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let wrong_version = EncryptedIdentity {
            version: ENCRYPTED_IDENTITY_VERSION + 1,
            ciphertext: "ncryptsec1invalid".to_string(),
        };
        let bytes = bincode::serialize(&wrong_version).unwrap();
        fs::write(dir.path().join("identity.bin"), bytes).unwrap();

        match store.load(&()).unwrap_err() {
            PersistenceError::VersionMismatch { expected, actual } => {
                assert_eq!(expected, ENCRYPTED_IDENTITY_VERSION);
                assert_eq!(actual, ENCRYPTED_IDENTITY_VERSION + 1);
            }
            other => panic!("unexpected error: {other}"),
        }
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
