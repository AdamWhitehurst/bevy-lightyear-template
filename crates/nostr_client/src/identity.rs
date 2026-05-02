use bevy::prelude::*;
use nostr_sdk::nips::nip49::EncryptedSecretKey;
use nostr_sdk::{FromBech32, Keys, PublicKey, SecretKey, ToBech32};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const ENCRYPTED_IDENTITY_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncryptedIdentity {
    pub version: u32,
    pub ciphertext: String,
}

#[derive(Resource, Clone)]
pub struct ClientIdentity {
    pub secret: SecretKey,
    pub public: PublicKey,
}

#[derive(Resource, Clone)]
pub struct ServerIdentity {
    pub keys: Keys,
}

impl ClientIdentity {
    pub fn from_secret(secret: SecretKey) -> Self {
        let keys = Keys::new(secret.clone());
        Self {
            secret,
            public: keys.public_key(),
        }
    }
}

pub fn client_id_from_public_key(public: &PublicKey) -> u64 {
    u64::from_le_bytes(
        public.as_bytes()[0..8]
            .try_into()
            .expect("public key has 32 bytes"),
    )
}

#[derive(Resource, Default, Clone, Debug)]
pub struct StoredEncryptedIdentity(pub Option<EncryptedIdentity>);

#[derive(Resource, Default, Clone, Debug)]
pub struct LoginError(pub Option<String>);

#[derive(Message, Clone, Debug)]
pub struct SaveEncryptedIdentity(pub EncryptedIdentity);

pub fn generate_encrypted_identity(
    passphrase: &str,
) -> Result<(ClientIdentity, EncryptedIdentity), String> {
    let secret = SecretKey::generate();
    encrypt_identity(secret, passphrase)
}

pub fn import_encrypted_identity(
    nsec: &str,
    passphrase: &str,
) -> Result<(ClientIdentity, EncryptedIdentity), String> {
    let secret = SecretKey::parse(nsec).map_err(|error| format!("invalid nsec: {error}"))?;
    encrypt_identity(secret, passphrase)
}

pub fn unlock_identity(
    encrypted: &EncryptedIdentity,
    passphrase: &str,
) -> Result<ClientIdentity, String> {
    if encrypted.version != ENCRYPTED_IDENTITY_VERSION {
        return Err(format!(
            "unsupported encrypted identity version {}",
            encrypted.version
        ));
    }

    let encrypted_key = EncryptedSecretKey::from_bech32(&encrypted.ciphertext)
        .map_err(|error| format!("invalid encrypted identity: {error}"))?;
    let secret = encrypted_key
        .decrypt(passphrase)
        .map_err(|error| format!("failed to decrypt identity: {error}"))?;
    Ok(ClientIdentity::from_secret(secret))
}

pub fn decode_nsec_or_ncryptsec(
    value: &str,
    passphrase: Option<&str>,
) -> Result<SecretKey, String> {
    let trimmed = value.trim();
    if trimmed.starts_with("ncryptsec1") {
        let passphrase = passphrase.ok_or("SERVER_NSEC_PASSPHRASE is required for ncryptsec")?;
        let encrypted = EncryptedSecretKey::from_bech32(trimmed)
            .map_err(|error| format!("invalid ncryptsec: {error}"))?;
        encrypted
            .decrypt(passphrase)
            .map_err(|error| format!("failed to decrypt ncryptsec: {error}"))
    } else {
        SecretKey::parse(trimmed).map_err(|error| format!("invalid nsec: {error}"))
    }
}

pub fn load_server_identity_from_env_or_file(
    path: Option<&Path>,
) -> Result<ServerIdentity, String> {
    let raw = match std::env::var("SERVER_NSEC") {
        Ok(value) => value,
        Err(_) => {
            let path = path.ok_or("SERVER_NSEC not set and no nsec_file_path configured")?;
            std::fs::read_to_string(path).map_err(|error| {
                format!(
                    "SERVER_NSEC not set and failed to read {}: {error}",
                    path.display()
                )
            })?
        }
    };
    let passphrase = std::env::var("SERVER_NSEC_PASSPHRASE").ok();
    let secret = decode_nsec_or_ncryptsec(&raw, passphrase.as_deref())?;
    Ok(ServerIdentity {
        keys: Keys::new(secret),
    })
}

pub fn nostr_config_dir() -> PathBuf {
    nostr_config_dir_from(
        non_empty_env_path("XDG_CONFIG_HOME"),
        non_empty_env_path("HOME"),
    )
}

pub fn client_identity_dir(profile: Option<&str>) -> Result<PathBuf, String> {
    match profile {
        Some(profile) => Ok(profile_config_dir(profile)?),
        None => Ok(nostr_config_dir()),
    }
}

pub fn server_nsec_file_path(profile: Option<&str>) -> Result<PathBuf, String> {
    match profile {
        Some(profile) => Ok(profile_config_dir(profile)?.join("server.nsec")),
        None => Ok(nostr_config_dir().join("server.nsec")),
    }
}

fn profile_config_dir(profile: &str) -> Result<PathBuf, String> {
    validate_identity_profile(profile)?;
    Ok(nostr_config_dir().join("profiles").join(profile))
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

fn encrypt_identity(
    secret: SecretKey,
    passphrase: &str,
) -> Result<(ClientIdentity, EncryptedIdentity), String> {
    let encrypted = secret
        .encrypt(passphrase)
        .map_err(|error| format!("failed to encrypt identity: {error}"))?;
    let identity = ClientIdentity::from_secret(secret);
    Ok((
        identity,
        EncryptedIdentity {
            version: ENCRYPTED_IDENTITY_VERSION,
            ciphertext: encrypted
                .to_bech32()
                .map_err(|error| format!("failed to encode ncryptsec: {error}"))?,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_identity_unlock_roundtrips() {
        let (identity, encrypted) = generate_encrypted_identity("correct horse").unwrap();
        let unlocked = unlock_identity(&encrypted, "correct horse").unwrap();

        assert_eq!(identity.public, unlocked.public);
    }

    #[test]
    fn wrong_passphrase_fails() {
        let (_identity, encrypted) = generate_encrypted_identity("correct horse").unwrap();

        assert!(unlock_identity(&encrypted, "wrong horse").is_err());
    }

    #[test]
    fn raw_nsec_decodes() {
        let secret = SecretKey::generate();
        let nsec = secret.to_bech32().unwrap();

        let decoded = decode_nsec_or_ncryptsec(&nsec, None).unwrap();

        assert_eq!(secret, decoded);
    }

    #[test]
    fn ncryptsec_decodes_with_passphrase() {
        let secret = SecretKey::generate();
        let encrypted = secret.encrypt("server passphrase").unwrap();
        let ncryptsec = encrypted.to_bech32().unwrap();

        let decoded = decode_nsec_or_ncryptsec(&ncryptsec, Some("server passphrase")).unwrap();

        assert_eq!(secret, decoded);
    }

    #[test]
    fn ncryptsec_requires_passphrase() {
        let secret = SecretKey::generate();
        let encrypted = secret.encrypt("server passphrase").unwrap();
        let ncryptsec = encrypted.to_bech32().unwrap();

        let error = decode_nsec_or_ncryptsec(&ncryptsec, None).unwrap_err();

        assert!(error.contains("SERVER_NSEC_PASSPHRASE"));
    }

    #[test]
    fn nostr_config_dir_prefers_xdg_config_home() {
        assert_eq!(
            nostr_config_dir_from(
                Some(PathBuf::from("/tmp/xdg")),
                Some(PathBuf::from("/tmp/home"))
            ),
            PathBuf::from("/tmp/xdg/nostr")
        );
    }

    #[test]
    fn nostr_config_dir_falls_back_to_home_config() {
        assert_eq!(
            nostr_config_dir_from(None, Some(PathBuf::from("/tmp/home"))),
            PathBuf::from("/tmp/home/.config/nostr")
        );
    }

    #[test]
    fn client_identity_dir_uses_profile_subdirectory() {
        assert_eq!(
            client_identity_dir(Some("alice_1")).unwrap(),
            nostr_config_dir().join("profiles").join("alice_1")
        );
    }

    #[test]
    fn server_nsec_file_path_uses_profile_subdirectory() {
        assert_eq!(
            server_nsec_file_path(Some("dev-server")).unwrap(),
            nostr_config_dir()
                .join("profiles")
                .join("dev-server")
                .join("server.nsec")
        );
    }

    #[test]
    fn identity_profile_rejects_path_like_values() {
        assert!(client_identity_dir(Some("../alice")).is_err());
        assert!(client_identity_dir(Some("alice/bob")).is_err());
        assert!(client_identity_dir(Some("")).is_err());
        assert!(client_identity_dir(Some("--client-id")).is_err());
    }
}
