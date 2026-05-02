pub mod announcement;
pub mod auth;
pub mod identity;
pub mod plugin;
pub mod relay_pool;

pub use announcement::{
    ServerAnnouncement, NOSTR_KIND_SERVER_ANNOUNCEMENT, SERVER_ANNOUNCEMENT_REPUBLISH_SECS,
    SERVER_ANNOUNCEMENT_TTL_SECS, SERVER_ANNOUNCEMENT_VERSION,
};
pub use auth::{build_identity_proof, verify_identity_proof, NOSTR_KIND_AUTH};
pub use identity::{
    client_id_from_public_key, client_identity_dir, decode_nsec_or_ncryptsec,
    generate_encrypted_identity, import_encrypted_identity, load_server_identity_from_env_or_file,
    server_nsec_file_path, unlock_identity, ClientIdentity, EncryptedIdentity, LoginError,
    SaveEncryptedIdentity, ServerIdentity, StoredEncryptedIdentity,
};
pub use plugin::{NostrClientConfig, NostrClientPlugin};
pub use relay_pool::{relay_pool_ready, RelayPool};
