use std::{fs, path::PathBuf, sync::Arc};

use nostr_client::{identity::ENCRYPTED_IDENTITY_VERSION, EncryptedIdentity};
use persistence::{PersistenceError, Store};

#[derive(Clone)]
pub struct FsEncryptedIdentityStore {
    pub base_dir: Arc<PathBuf>,
}

pub fn default_nostr_identity_dir() -> PathBuf {
    nostr_identity_dir(None).expect("default identity profile is valid")
}

pub fn nostr_identity_dir(profile: Option<&str>) -> Result<PathBuf, String> {
    match profile {
        Some(profile) => {
            validate_identity_profile(profile)?;
            Ok(nostr_config_dir().join("profiles").join(profile))
        }
        None => Ok(nostr_config_dir()),
    }
}

fn validate_identity_profile(profile: &str) -> Result<(), String> {
    if profile.is_empty() {
        return Err("Nostr identity profile must not be empty".to_string());
    }
    let mut bytes = profile.bytes();
    let first = bytes.next().expect("profile is non-empty");
    if !first.is_ascii_alphanumeric() {
        return Err(format!(
            "Nostr identity profile '{profile}' must start with an ASCII letter or digit"
        ));
    }
    if bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')) {
        Ok(())
    } else {
        Err(format!(
            "Nostr identity profile '{profile}' must contain only ASCII letters, digits, '.', '_', or '-'"
        ))
    }
}

fn nostr_config_dir() -> PathBuf {
    nostr_config_dir_from(
        non_empty_env_path("XDG_CONFIG_HOME"),
        non_empty_env_path("HOME"),
    )
}

fn nostr_config_dir_from(xdg_config_home: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    if let Some(path) = xdg_config_home {
        return path.join("nostr");
    }
    if let Some(home) = home {
        return home.join(".config").join("nostr");
    }
    PathBuf::from(".config").join("nostr")
}

fn non_empty_env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
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

    #[test]
    fn default_identity_dir_prefers_xdg_config_home() {
        assert_eq!(
            nostr_config_dir_from(
                Some(PathBuf::from("/tmp/xdg")),
                Some(PathBuf::from("/tmp/home"))
            ),
            PathBuf::from("/tmp/xdg/nostr")
        );
    }

    #[test]
    fn default_identity_dir_falls_back_to_home_config() {
        assert_eq!(
            nostr_config_dir_from(None, Some(PathBuf::from("/tmp/home"))),
            PathBuf::from("/tmp/home/.config/nostr")
        );
    }

    #[test]
    fn named_identity_dir_uses_profile_subdirectory() {
        assert_eq!(
            nostr_identity_dir(Some("alice_1")).unwrap(),
            nostr_config_dir().join("profiles").join("alice_1")
        );
    }

    #[test]
    fn named_identity_dir_rejects_paths() {
        assert!(nostr_identity_dir(Some("../alice")).is_err());
        assert!(nostr_identity_dir(Some("alice/bob")).is_err());
        assert!(nostr_identity_dir(Some("")).is_err());
        assert!(nostr_identity_dir(Some("--client-id")).is_err());
    }

    fn test_store(dir: &std::path::Path) -> FsEncryptedIdentityStore {
        FsEncryptedIdentityStore {
            base_dir: Arc::new(dir.to_path_buf()),
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
