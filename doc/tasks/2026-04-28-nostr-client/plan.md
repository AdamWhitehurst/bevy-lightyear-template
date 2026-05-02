# Implementation Plan

## Overview

A new `nostr_client` workspace crate provides identity, signing, relay-pool management, and Nostr event read/write to both native client, web client, and server. The app gates `AppState::Ready` on assets plus Nostr relay readiness and identity-load readiness; client login, server discovery, post-connect identity proof, and homebase ownership all use durable Nostr identity instead of transient Lightyear `RemoteId` bits.

## Global implementation rules

- Follow phases in order. Each phase must compile and be manually runnable before starting the next phase.
- If implementation discovers a required behavioral or structural change not listed here, stop and get user approval before changing the plan.
- Approved structure deviation: keep `protocol` free of `nostr-sdk`/Nostr dependencies. `protocol` owns wire-safe domain types (`NostrPublicKey`, `IdentityProof.signed_event_json`), and `nostr_client` converts to/from `nostr_sdk::{PublicKey, Event}` at crate boundaries.
- Text input crate choice: use `bevy_ui_text_input = "0.7"` instead of `bevy_simple_text_input`; `bevy_simple_text_input 0.14.x` matches Bevy 0.18 but explicitly does not support paste, while import flow requires paste. This is within structure.md's "or vetted alternative" allowance.
- Nostr public keys are x-only 32-byte keys. Derive Lightyear `client_id` with `u64::from_le_bytes(public_key.as_bytes()[0..8].try_into().unwrap())`; do not skip a parity byte.
- Before any `cargo check`, `cargo build`, `cargo test`, `cargo client*`, `cargo server*`, or `wasm-pack test`, check for an already running build/test command and wait or stop it; project policy forbids parallel cargo builds/tests.
- Use cargo aliases from `.cargo/config.toml`: `cargo check-all`, `cargo test-native`, `cargo web-build`, `cargo client-log`, `cargo server-log`.
- Agent must not run `cargo client`, `cargo server`, `cargo client-log`, or `cargo server-log`; the user owns all client/server manual run commands.
- Do not add save-format migration or compatibility shims. Phases 5 and 6 are wire/save-format breaks by design.

---

## Phase 1: `nostr_client` crate + relay pool + Loading gate

### Changes

#### 1. Workspace membership and dependencies
**File**: `Cargo.toml`  
**Action**: modify

Add the new crate to `[workspace].members` and add a workspace dependency for consumers.

```toml
[workspace]
members = [
  "crates/protocol",
  "crates/client",
  # ...existing members...
  "crates/persistence",
  "crates/nostr_client",
]

[workspace.dependencies]
nostr_client = { path = "crates/nostr_client" }
```

Do not add `nostr-sdk` to `protocol`.

#### 2. New Nostr client crate manifest
**File**: `crates/nostr_client/Cargo.toml`  
**Action**: create

Start with the SDK and bridge dependencies needed by native and WASM builds. Do not wrap the Nostr relay task in `async_compat`: `nostr-sdk`/`nostr-relay-pool` already use wasm-safe browser primitives (`web_sys`, `wasm_bindgen_futures`, `gloo_timers`) on wasm and their own runtime bridge on native. If `cargo web-build` fails because of SDK feature selection, fix feature-gating in this phase before proceeding.

```toml
[package]
name = "nostr_client"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true, features = ["bevy_log"] }
protocol = { workspace = true }
async-channel = "2"
futures-lite = "2"
nostr-sdk = "0.44"
serde = { workspace = true, features = ["derive"] }
serde_json = "1"
hex = "0.4"
```

#### 3. Crate public surface
**File**: `crates/nostr_client/src/lib.rs`  
**Action**: create

Expose only stable ECS/domain APIs. Keep SDK details inside modules unless callers must sign/verify.

```rust
pub mod identity;
pub mod plugin;
pub mod relay_pool;

pub use plugin::{NostrClientConfig, NostrClientPlugin};
pub use relay_pool::{relay_pool_ready, RelayPool};
```

`announcement` is added in Phase 3.

#### 4. Relay pool resource and startup bridge
**File**: `crates/nostr_client/src/relay_pool.rs`  
**Action**: create

Create a long-lived `nostr_sdk::Client` inside a Bevy `IoTaskPool` task and bridge readiness into ECS through `async_channel`. Readiness means at least one configured relay reaches EOSE on the discovery subscription.

```rust
use async_channel::Receiver;
use bevy::prelude::*;
use nostr_sdk::{Client, Filter};
use protocol::RelayPoolReady;

#[derive(Resource, Clone)]
pub struct RelayPool {
    pub client: Client,
    pub ready_rx: Receiver<()>,
}


pub fn relay_pool_ready(pool: Res<RelayPoolReady>) -> bool {
    pool.0
}

pub fn poll_relay_pool_ready(mut ready: ResMut<RelayPoolReady>, pool: Option<Res<RelayPool>>) {
    let Some(pool) = pool else {
        trace!("poll_relay_pool_ready: RelayPool not inserted yet");
        return;
    };
    while pool.ready_rx.try_recv().is_ok() {
        if !ready.0 {
            info!("Nostr relay pool reached EOSE on at least one relay");
        }
        ready.0 = true;
    }
}
```

Startup helper shape:

```rust
pub fn spawn_relay_pool(mut commands: Commands, config: Res<NostrClientConfig>) {
    let (ready_tx, ready_rx) = async_channel::bounded(1);
    let client = nostr_sdk::Client::default();

    commands.insert_resource(RelayPool {
        client: client.clone(),
        ready_rx,
    });

    let relays = config.relays.clone();
    IoTaskPool::get().spawn(async move {
        for relay in relays {
            match client.add_relay(relay.clone()).await {
                Ok(_) => debug!(%relay, "added Nostr relay"),
                Err(error) => warn!(%relay, %error, "failed to add Nostr relay"),
            }
        }
        client.connect().await;
        let filter = Filter::new().limit(1);
        let subscription = client.subscribe(filter, None).await;
        debug!(?subscription, "started Nostr readiness subscription");

        // Use client.notifications()/RelayPoolNotification and send ready_tx exactly once
        // when the subscription observes EOSE from any relay.
        // Match exact notification variant names against nostr-sdk 0.44 during implementation.
    }).detach();
}
```

Also add a shutdown system in `Last`:

```rust
pub fn shutdown_relay_pool(pool: Option<Res<RelayPool>>) {
    let Some(pool) = pool else {
        trace!("shutdown_relay_pool: RelayPool absent, nothing to shut down");
        return;
    };
    let client = pool.client.clone();
    IoTaskPool::get()
        .spawn(async move {
            client.shutdown().await;
        })
        .detach();
}
```

#### 5. Nostr plugin and relay config
**File**: `crates/nostr_client/src/plugin.rs`  
**Action**: create

Add plugin shape matching `ClientNetworkPlugin`: clone config into a resource, initialize readiness resource, spawn startup setup, poll in `Update`, drain shutdown in `Last`.

```rust
use bevy::prelude::*;
use protocol::RelayPoolReady;
use crate::relay_pool::{poll_relay_pool_ready, shutdown_relay_pool, spawn_relay_pool};

#[derive(Clone, Resource, Debug)]
pub struct NostrClientConfig {
    pub relays: Vec<String>,
}

impl Default for NostrClientConfig {
    fn default() -> Self {
        Self { relays: relays_from_env_or_default() }
    }
}

fn relays_from_env_or_default() -> Vec<String> {
    std::env::var("NOSTR_RELAYS")
        .ok()
        .map(|raw| raw.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect())
        .filter(|relays: &Vec<String>| !relays.is_empty())
        .unwrap_or_else(|| vec![
            "wss://relay.damus.io".to_string(),
            "wss://nos.lol".to_string(),
            "wss://relay.primal.net".to_string(),
        ])
}

pub struct NostrClientPlugin {
    pub config: NostrClientConfig,
}

impl Default for NostrClientPlugin {
    fn default() -> Self { Self { config: NostrClientConfig::default() } }
}

impl Plugin for NostrClientPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config.clone())
            .init_resource::<RelayPoolReady>()
            .add_systems(Startup, spawn_relay_pool)
            .add_systems(Update, poll_relay_pool_ready)
            .add_systems(Last, shutdown_relay_pool);
    }
}
```

#### 6. Pre-Ready loading gate
**File**: `crates/protocol/src/app_state.rs`  
**Action**: modify

Add protocol-owned readiness resources so `protocol` does not depend on `nostr_client`.

```rust
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct RelayPoolReady(pub bool);

#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct IdentityLoadComplete(pub bool);
```

Initialize both in `AppStatePlugin` and AND them into `check_assets_loaded`.

```rust
impl Plugin for AppStatePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>();
        app.init_resource::<TrackedAssets>();
        app.init_resource::<RelayPoolReady>();
        app.init_resource::<IdentityLoadComplete>();
        app.add_systems(Update, check_assets_loaded.run_if(in_state(AppState::Loading)));
    }
}

fn check_assets_loaded(
    asset_server: Res<AssetServer>,
    tracked: Res<TrackedAssets>,
    relay_ready: Res<RelayPoolReady>,
    identity_ready: Res<IdentityLoadComplete>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let assets_loaded = tracked.0.iter().all(|handle| asset_server.is_loaded_with_dependencies(handle));
    if !assets_loaded {
        trace!("check_assets_loaded: tracked assets still loading");
        return;
    }
    if !relay_ready.0 {
        trace!("check_assets_loaded: waiting for Nostr relay EOSE");
        return;
    }
    if !identity_ready.0 {
        trace!("check_assets_loaded: waiting for identity store load");
        return;
    }

    info!("Startup gates complete, transitioning to AppState::Ready");
    next_state.set(AppState::Ready);
}
```

In Phase 1 only, set `IdentityLoadComplete(true)` by default in the Nostr plugin startup path so the new gate does not block until Phase 2 implements real identity loading.

#### 6a. Client UI startup state gate
**File**: `crates/ui/src/state.rs`  
**Action**: modify

`ClientState` must not default to an interactive state. Add a startup-only `Loading` state as the default so the UI cannot show `MainMenu`, connect, or enter `InGame` while global `AppState::Loading` is waiting on assets, relay EOSE, or identity load.

```rust
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, States)]
pub enum ClientState {
    #[default]
    Loading,
    MainMenu,
    Connecting,
    InGame,
}
```

**File**: `crates/ui/src/lib.rs`  
**Action**: modify

Add one startup bridge from global readiness into the UI state machine: when `AppState::Ready` is entered and `ClientState` is still `Loading`, transition to `ClientState::MainMenu`. Do not broadly gate unrelated UI systems on `AppState::Ready`; the single state transition owns the startup boundary.

Also make `on_client_connected` transition to `InGame` only when the current `ClientState` is `Connecting`, so early network connections cannot bypass the startup gate.


**File**: `crates/protocol/src/lib.rs`  
**Action**: modify

Re-export the new gate resources for client/server startup systems.

```rust
pub use app_state::{
    AppState, AppStatePlugin, IdentityLoadComplete, RelayPoolReady, TrackedAssets,
};
```

#### 7. Native client wiring
**File**: `crates/client/Cargo.toml`  
**Action**: modify

```toml
nostr_client = { workspace = true }
```

**File**: `crates/client/src/main.rs`  
**Action**: modify

Add `NostrClientPlugin` before `SharedGameplayPlugin` or immediately after it; it only contributes startup gates and resources.

```rust
use nostr_client::NostrClientPlugin;

// ...
.add_plugins(SharedGameplayPlugin)
.add_plugins(NostrClientPlugin::default())
.add_plugins(ClientNetworkPlugin { config: network_config })
```

#### 8. Server wiring
**File**: `crates/server/Cargo.toml`  
**Action**: modify

```toml
nostr_client = { workspace = true }
```

**File**: `crates/server/src/main.rs`  
**Action**: modify

```rust
use nostr_client::NostrClientPlugin;

// ...
.add_plugins(SharedGameplayPlugin)
.add_plugins(NostrClientPlugin::default())
.add_plugins(ServerNetworkPlugin { config: ServerNetworkConfig { /* existing */ } })
```

#### 9. Web client wiring
**File**: `crates/web/Cargo.toml`  
**Action**: modify

```toml
nostr_client = { workspace = true }
```

**File**: `crates/web/src/main.rs`  
**Action**: modify

```rust
use nostr_client::NostrClientPlugin;

// ...
.add_plugins(SharedGameplayPlugin)
.add_plugins(NostrClientPlugin::default())
.add_plugins(WebClientPlugin::default())
```

### Verification
#### Automated
- [x] `pgrep -af 'cargo (build|check|test|run)|wasm-pack test'` shows no other active build/test before running cargo commands.
- [x] `cargo check-all` passes.
- [x] `cargo test-native` passes.
- [x] `cargo web-build` passes.
- [x] `RUST_LOG=nostr_client=debug cargo client-log` starts and logs relay setup plus an EOSE/readiness log before the `AppState::Ready` transition.

#### Manual
- [x] Run `RUST_LOG=nostr_client=debug cargo client-log`; observe a Compat-spawned Nostr setup task, at least one relay EOSE/readiness message, then `AppState::Ready`.
- [x] Run `NOSTR_RELAYS=wss://does-not-exist.invalid RUST_LOG=nostr_client=debug cargo client-log`; observe the app stays in `Loading` and logs that it is waiting for Nostr relay EOSE.
- [x] Restore valid relays with `unset NOSTR_RELAYS` or valid `NOSTR_RELAYS=...`; observe startup progresses again.

---

## Phase 2: Identity persistence + `ClientState::Login`

### Changes

#### 1. Identity domain and NIP-49 helpers
**File**: `crates/nostr_client/src/identity.rs`  
**Action**: create

Define persisted ciphertext, in-memory identity, and conversion helpers. Plaintext never leaves `ClientIdentity` except for signing/encryption helpers.

```rust
use bevy::prelude::*;
use nostr_sdk::{EncryptedSecretKey, Keys, PublicKey, SecretKey, ToBech32, FromBech32};
use serde::{Deserialize, Serialize};

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

impl ClientIdentity {
    pub fn from_secret(secret: SecretKey) -> Self {
        let keys = Keys::new(secret.clone());
        Self { secret, public: keys.public_key() }
    }
}

pub fn client_id_from_public_key(public: &PublicKey) -> u64 {
    u64::from_le_bytes(public.as_bytes()[0..8].try_into().expect("public key has 32 bytes"))
}

#[derive(Resource, Default, Clone, Debug)]
pub struct StoredEncryptedIdentity(pub Option<EncryptedIdentity>);

#[derive(Resource, Default, Clone, Debug)]
pub struct LoginError(pub Option<String>);

#[derive(Message, Clone, Debug)]
pub struct SaveEncryptedIdentity(pub EncryptedIdentity);

pub fn generate_encrypted_identity(passphrase: &str) -> Result<(ClientIdentity, EncryptedIdentity), String> {
    let secret = SecretKey::generate();
    encrypt_identity(secret, passphrase)
}

pub fn import_encrypted_identity(nsec: &str, passphrase: &str) -> Result<(ClientIdentity, EncryptedIdentity), String> {
    let secret = SecretKey::parse(nsec).map_err(|e| format!("invalid nsec: {e}"))?;
    encrypt_identity(secret, passphrase)
}

pub fn unlock_identity(encrypted: &EncryptedIdentity, passphrase: &str) -> Result<ClientIdentity, String> {
    if encrypted.version != ENCRYPTED_IDENTITY_VERSION {
        return Err(format!("unsupported encrypted identity version {}", encrypted.version));
    }
    let encrypted_key = EncryptedSecretKey::from_bech32(&encrypted.ciphertext)
        .map_err(|e| format!("invalid encrypted identity: {e}"))?;
    let secret = encrypted_key.decrypt(passphrase)
        .map_err(|e| format!("failed to decrypt identity: {e}"))?;
    Ok(ClientIdentity::from_secret(secret))
}

fn encrypt_identity(secret: SecretKey, passphrase: &str) -> Result<(ClientIdentity, EncryptedIdentity), String> {
    let encrypted = secret.encrypt(passphrase)
        .map_err(|e| format!("failed to encrypt identity: {e}"))?;
    let identity = ClientIdentity::from_secret(secret);
    Ok((identity, EncryptedIdentity {
        version: ENCRYPTED_IDENTITY_VERSION,
        ciphertext: encrypted.to_bech32().map_err(|e| format!("failed to encode ncryptsec: {e}"))?,
    }))
}
```

Add tests in this file:

```rust
#[test]
fn encrypted_identity_unlock_roundtrips() { /* generate -> unlock -> same public */ }

#[test]
fn wrong_passphrase_fails() { /* generate -> unlock with wrong passphrase -> Err */ }
```

#### 2. Export identity APIs
**File**: `crates/nostr_client/src/lib.rs`  
**Action**: modify

```rust
pub mod identity;
pub use identity::{
    client_id_from_public_key, ClientIdentity, EncryptedIdentity, LoginError, SaveEncryptedIdentity,
    StoredEncryptedIdentity, generate_encrypted_identity, import_encrypted_identity, unlock_identity,
};
```

#### 3. Native encrypted identity store
**File**: `crates/client/src/persistence/mod.rs`  
**Action**: create

```rust
pub mod fs_encrypted_identity;
```

**File**: `crates/client/src/persistence/fs_encrypted_identity.rs`  
**Action**: create

Mirror `FsMapMetaStore`, writing `worlds/identity.bin.tmp` then rename to `worlds/identity.bin`.

```rust
use std::{fs, path::PathBuf, sync::Arc};
use nostr_client::{EncryptedIdentity, identity::ENCRYPTED_IDENTITY_VERSION};
use persistence::{PersistenceError, Store};

#[derive(Clone)]
pub struct FsEncryptedIdentityStore {
    pub base_dir: Arc<PathBuf>,
}

impl Store<(), EncryptedIdentity> for FsEncryptedIdentityStore {
    fn save(&self, _key: &(), value: &EncryptedIdentity) -> Result<(), PersistenceError> {
        fs::create_dir_all(self.base_dir.as_ref())
            .map_err(|e| PersistenceError::Serialize(format!("mkdir identity dir: {e}")))?;
        let path = self.base_dir.join("identity.bin");
        let bytes = bincode::serialize(value)
            .map_err(|e| PersistenceError::Serialize(format!("serialize identity: {e}")))?;
        let tmp_path = path.with_extension("bin.tmp");
        fs::write(&tmp_path, &bytes)
            .map_err(|e| PersistenceError::Serialize(format!("write identity tmp: {e}")))?;
        fs::rename(&tmp_path, &path)
            .map_err(|e| PersistenceError::Serialize(format!("rename identity: {e}")))?;
        Ok(())
    }

    fn load(&self, _key: &()) -> Result<Option<EncryptedIdentity>, PersistenceError> {
        let path = self.base_dir.join("identity.bin");
        if !path.exists() { return Ok(None); }
        let bytes = fs::read(&path)
            .map_err(|e| PersistenceError::Deserialize(format!("read identity: {e}")))?;
        let identity: EncryptedIdentity = bincode::deserialize(&bytes)
            .map_err(|e| PersistenceError::Deserialize(format!("deserialize identity: {e}")))?;
        if identity.version != ENCRYPTED_IDENTITY_VERSION {
            return Err(PersistenceError::VersionMismatch {
                expected: ENCRYPTED_IDENTITY_VERSION,
                actual: identity.version,
            });
        }
        Ok(Some(identity))
    }
}
```

Add unit tests for missing file, roundtrip, wrong version, and atomic file path.

#### 4. Client crate module and dependencies
**File**: `crates/client/Cargo.toml`  
**Action**: modify

```toml
persistence = { workspace = true }
bincode = "1.3"
nostr_client = { workspace = true }
```

**File**: `crates/client/src/lib.rs`  
**Action**: modify

```rust
pub mod persistence;
```

#### 5. Identity store entity and load polling
**File**: `crates/client/src/main.rs`  
**Action**: modify

Spawn a singleton persistence entity on native client startup and initiate one load for key `()`.

```rust
use client::persistence::fs_encrypted_identity::FsEncryptedIdentityStore;
use nostr_client::{EncryptedIdentity, SaveEncryptedIdentity, StoredEncryptedIdentity, LoginError};
use persistence::{PendingStoreOps, StoreBackend};
use protocol::IdentityLoadComplete;
use std::{path::PathBuf, sync::Arc};

fn spawn_identity_store(mut commands: Commands) {
    let store = FsEncryptedIdentityStore { base_dir: Arc::new(PathBuf::from("worlds")) };
    let mut ops = PendingStoreOps::<(), EncryptedIdentity>::default();
    ops.spawn_load(&store, ());
    commands.spawn((
        Name::new("Encrypted Identity Store"),
        StoreBackend::new(store),
        ops,
    ));
}

fn poll_identity_store_load(
    mut query: Query<(&StoreBackend<(), EncryptedIdentity, FsEncryptedIdentityStore>, &mut PendingStoreOps<(), EncryptedIdentity>)>,
    mut stored: ResMut<StoredEncryptedIdentity>,
    mut complete: ResMut<IdentityLoadComplete>,
) {
    let Ok((_store, mut ops)) = query.single_mut() else {
        trace!("poll_identity_store_load: identity store entity not spawned yet");
        return;
    };
    ops.poll();
    for (_key, loaded) in ops.completed_loads.drain(..) {
        stored.0 = loaded;
        complete.0 = true;
        info!("Encrypted identity load complete");
    }
    for (_key, error) in ops.load_errors.drain(..) {
        panic!("Failed to load encrypted identity: {error}");
    }
}

fn handle_identity_save_requests(
    mut requests: MessageReader<SaveEncryptedIdentity>,
    mut query: Query<(&StoreBackend<(), EncryptedIdentity, FsEncryptedIdentityStore>, &mut PendingStoreOps<(), EncryptedIdentity>)>,
) {
    let Ok((store, mut ops)) = query.single_mut() else {
        trace!("handle_identity_save_requests: identity store entity not spawned yet");
        return;
    };
    for request in requests.read() {
        ops.spawn_save(&store.0, (), request.0.clone());
    }
}
```

Register resources/systems:

```rust
.add_message::<SaveEncryptedIdentity>()
.init_resource::<StoredEncryptedIdentity>()
.init_resource::<LoginError>()
.add_systems(Startup, spawn_identity_store)
.add_systems(Update, (poll_identity_store_load, handle_identity_save_requests))
```

Remove the Phase 1 native placeholder that set `IdentityLoadComplete(true)`. After this phase, native clients set `IdentityLoadComplete(true)` only from `poll_identity_store_load`; web/session-only clients may keep the explicit ready default because no web persistence is in scope.

For web, Phase 1's default `IdentityLoadComplete(true)` remains the v1 behavior: no web persistence, prompt each session.

#### 6. UI dependencies
**File**: `crates/ui/Cargo.toml`  
**Action**: modify

```toml
nostr_client = { workspace = true }
bevy_ui_text_input = "0.7"
```

Native save requests are emitted as `SaveEncryptedIdentity` messages and handled in `client/src/main.rs`; `ui` must not depend on the `client` crate or filesystem store types.

#### 7. Login widgets helper module
**File**: `crates/ui/src/widgets.rs`  
**Action**: create

Keep helpers scoped to new login/server-list screens. Do not retrofit existing UI.

```rust
use bevy::prelude::*;
use bevy_ui_text_input::{TextInputBuffer, TextInputMode, TextInputNode, TextInputPrompt};

pub const SCREEN_BG: Color = Color::srgb(0.1, 0.1, 0.1);
pub const BUTTON_BG: Color = Color::srgb(0.2, 0.2, 0.2);
pub const TEXT_COLOR: Color = Color::WHITE;
pub const BUTTON_SIZE: Vec2 = Vec2::new(240.0, 65.0);

pub fn spawn_button<M: Component>(parent: &mut ChildSpawnerCommands, label: &str, marker: M) {
    parent.spawn((
        Button,
        Node {
            width: Val::Px(BUTTON_SIZE.x),
            height: Val::Px(BUTTON_SIZE.y),
            border: UiRect::all(Val::Px(5.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BorderColor::all(TEXT_COLOR),
        BackgroundColor(BUTTON_BG),
        marker,
    )).with_children(|parent| {
        parent.spawn((Text::new(label), TextFont { font_size: 33.0, ..default() }, TextColor(TEXT_COLOR)));
    });
}

pub fn spawn_text_input<M: Component>(parent: &mut ChildSpawnerCommands, placeholder: &str, marker: M, _password: bool) {
    parent.spawn((
        TextInputNode::default(),
        TextInputBuffer::default(),
        TextInputPrompt::new(placeholder),
        TextInputMode::SingleLine,
        Node { width: Val::Px(520.0), height: Val::Px(48.0), ..default() },
        marker,
    ));
}
```

`bevy_ui_text_input` 0.7 provides `TextInputPlugin`, `TextInputNode`, `TextInputBuffer::get_text()`, `TextInputPrompt::new`, and `TextInputMode::SingleLine`; use these APIs directly and preserve this helper boundary.

#### 8. Login state
**File**: `crates/ui/src/state.rs`  
**Action**: modify

```rust
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, States)]
pub enum ClientState {
    #[default]
    Loading,
    Login,
    MainMenu,
    Connecting,
    InGame,
}
```

Phase 2 changes the `AppState::Ready` startup bridge added in Phase 1 from `Loading -> MainMenu` to `Loading -> Login`; `Loading` remains the only default initial client state.

#### 9. Login and input marker components
**File**: `crates/ui/src/components.rs`  
**Action**: modify

```rust
#[derive(Component)]
pub struct GenerateButton;

#[derive(Component)]
pub struct ImportButton;

#[derive(Component)]
pub struct UnlockButton;

#[derive(Component)]
pub struct PassphraseInput;

#[derive(Component)]
pub struct NsecInput;
```

#### 10. Login screen systems
**File**: `crates/ui/src/login.rs`  
**Action**: create

Branch on stored ciphertext presence. Missing identity shows Generate + Import. Existing identity shows Unlock. All flows insert `ClientIdentity` on success and transition to `MainMenu`.

```rust
use bevy::prelude::*;
use nostr_client::{
    generate_encrypted_identity, import_encrypted_identity, unlock_identity,
    ClientIdentity, LoginError, SaveEncryptedIdentity, StoredEncryptedIdentity,
};
use crate::{components::*, state::ClientState, widgets};

pub fn setup_login_screen(
    mut commands: Commands,
    stored: Res<StoredEncryptedIdentity>,
    error: Res<LoginError>,
) {
    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(16.0),
            ..default()
        },
        BackgroundColor(widgets::SCREEN_BG),
        DespawnOnExit(ClientState::Login),
    )).with_children(|parent| {
        parent.spawn((Text::new("Nostr Login"), TextFont { font_size: 60.0, ..default() }, TextColor(Color::WHITE)));
        if let Some(message) = &error.0 {
            parent.spawn((Text::new(message.clone()), TextFont { font_size: 24.0, ..default() }, TextColor(Color::srgb(1.0, 0.2, 0.2))));
        }
        widgets::spawn_text_input(parent, "passphrase", PassphraseInput, true);
        if stored.0.is_some() {
            widgets::spawn_button(parent, "Unlock", UnlockButton);
        } else {
            widgets::spawn_text_input(parent, "nsec1...", NsecInput, false);
            widgets::spawn_button(parent, "Generate", GenerateButton);
            widgets::spawn_button(parent, "Import", ImportButton);
        }
    });
}
```

Interaction system shape:

```rust
pub fn login_button_interaction(
    mut commands: Commands,
    mut next_state: ResMut<NextState<ClientState>>,
    mut stored: ResMut<StoredEncryptedIdentity>,
    mut error: ResMut<LoginError>,
    generate: Query<&Interaction, (Changed<Interaction>, With<GenerateButton>)>,
    import: Query<&Interaction, (Changed<Interaction>, With<ImportButton>)>,
    unlock: Query<&Interaction, (Changed<Interaction>, With<UnlockButton>)>,
    passphrase: Query<&TextInputBuffer, With<PassphraseInput>>,
    nsec: Query<&TextInputBuffer, With<NsecInput>>,
    mut save_writer: MessageWriter<SaveEncryptedIdentity>,
) {
    let passphrase = passphrase.single().expect("PassphraseInput must exist").get_text();
    let result = if pressed(&generate) {
        generate_encrypted_identity(&passphrase)
    } else if pressed(&import) {
        let nsec = nsec.single().expect("NsecInput must exist").get_text();
        import_encrypted_identity(&nsec, &passphrase)
    } else if pressed(&unlock) {
        let encrypted = stored.0.as_ref().expect("stored identity required for unlock");
        unlock_identity(encrypted, &passphrase).map(|identity| (identity, encrypted.clone()))
    } else {
        return;
    };

    match result {
        Ok((identity, encrypted)) => {
            commands.insert_resource(identity);
            stored.0 = Some(encrypted.clone());
            save_writer.write(SaveEncryptedIdentity(encrypted));
            error.0 = None;
            next_state.set(ClientState::MainMenu);
        }
        Err(message) => {
            warn!(%message, "login failed");
            error.0 = Some(message);
        }
    }
}
```

Native persistence is handled by `client/src/main.rs` through `SaveEncryptedIdentity`; `ui` must not depend on the `client` crate or filesystem store types.

#### 11. UI plugin registration
**File**: `crates/ui/src/lib.rs`  
**Action**: modify

Add modules and plugin dependency.

```rust
pub mod login;
pub mod widgets;

use bevy_ui_text_input::TextInputPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TextInputPlugin);
        app.init_resource::<LoginError>();
        app.add_systems(OnEnter(ClientState::Login), login::setup_login_screen);
        app.add_systems(Update, login::login_button_interaction.run_if(in_state(ClientState::Login)));
        // existing systems unchanged
    }
}
```

Adjust `on_client_disconnected` so disconnect returns to `MainMenu` only if a `ClientIdentity` exists; otherwise return to `Login`.

### Verification
#### Automated
- [x] `pgrep -af 'cargo (build|check|test|run)|wasm-pack test'` shows no other active build/test before running cargo commands.
- [x] `cargo check-all` passes.
- [x] `cargo test-native` passes.
- [x] `cargo test -p nostr_client identity` passes.
- [x] `cargo test -p client fs_encrypted_identity` passes.
- [x] `cargo web-build` passes after removing `async_compat` from `nostr_client`.
- [x] `bevy run web -p 4001` plus browser reload reaches `AppState::Ready`/`ClientState::Login` without the `async-compat` wasm panic.

#### Manual
- [x] Delete or move `worlds/identity.bin`, then run `cargo client`; app opens to `Login` with Generate and Import choices.
- [x] Generate flow: enter passphrase, click Generate; app transitions to `MainMenu`, and `worlds/identity.bin` exists with no plaintext `nsec1` string visible when inspected as binary.
- [x] Restart client; Unlock screen appears; correct passphrase transitions to `MainMenu`.
- [x] Wrong passphrase shows error feedback and remains in `Login`.
- [x] Import flow: remove `worlds/identity.bin`, paste an `nsec1...`, enter passphrase, click Import; app stores encrypted identity and transitions to `MainMenu`.
- [x] Web build still passes; web client reaches Login and can use session-only Generate/Import without writing `worlds/identity.bin`.

---

## Phase 3: Server identity + announcement publishing

### Changes

#### 1. Server config fallback path
**File**: `crates/server_lightyear/src/connection.rs`  
**Action**: modify

Add server Nostr key fallback file path to network config.

```rust
#[derive(Clone, Resource)]
pub struct ServerNetworkConfig {
    pub bind_addr: IpAddr,
    pub port: u16,
    pub protocol_id: u64,
    pub private_key: [u8; 32],
    pub cert_pem_path: PathBuf,
    pub key_pem_path: PathBuf,
    pub nsec_file_path: Option<PathBuf>,
    pub replication_interval: Duration,
}

impl Default for ServerNetworkConfig {
    fn default() -> Self {
        Self {
            // existing fields...
            nsec_file_path: Some(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../keys/server.nsec"))),
            replication_interval: REPLICATION_INTERVAL,
        }
    }
}
```

#### 2. Identity decode helpers
**File**: `crates/nostr_client/src/identity.rs`  
**Action**: modify

Extend with server decode and `ServerIdentity`.

```rust
#[derive(Resource, Clone)]
pub struct ServerIdentity {
    pub keys: Keys,
}

pub fn decode_nsec_or_ncryptsec(value: &str, passphrase: Option<&str>) -> Result<SecretKey, String> {
    let trimmed = value.trim();
    if trimmed.starts_with("ncryptsec1") {
        let passphrase = passphrase.ok_or("SERVER_NSEC_PASSPHRASE is required for ncryptsec")?;
        let encrypted = EncryptedSecretKey::from_bech32(trimmed)
            .map_err(|e| format!("invalid ncryptsec: {e}"))?;
        encrypted.decrypt(passphrase).map_err(|e| format!("failed to decrypt ncryptsec: {e}"))
    } else {
        SecretKey::parse(trimmed).map_err(|e| format!("invalid nsec: {e}"))
    }
}

pub fn load_server_identity_from_env_or_file(path: Option<&Path>) -> Result<ServerIdentity, String> {
    let raw = match std::env::var("SERVER_NSEC") {
        Ok(value) => value,
        Err(_) => {
            let path = path.ok_or("SERVER_NSEC not set and no nsec_file_path configured")?;
            std::fs::read_to_string(path)
                .map_err(|e| format!("SERVER_NSEC not set and failed to read {}: {e}", path.display()))?
        }
    };
    let passphrase = std::env::var("SERVER_NSEC_PASSPHRASE").ok();
    let secret = decode_nsec_or_ncryptsec(&raw, passphrase.as_deref())?;
    Ok(ServerIdentity { keys: Keys::new(secret) })
}
```

#### 3. Announcement shared helpers
**File**: `crates/nostr_client/src/announcement.rs`  
**Action**: create

Define the event kind, payload, builder, and async publish helper. Keep actual `nostr_sdk` usage in this crate.

```rust
use std::net::SocketAddr;
use bevy::prelude::*;
use nostr_sdk::{Client, EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};

pub const NOSTR_KIND_SERVER_ANNOUNCEMENT: u16 = 30078;
pub const SERVER_ANNOUNCEMENT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerAnnouncement {
    pub server_addr: SocketAddr,
    pub cert_digest: String,
    pub display_name: String,
    pub version: u32,
}

pub fn server_announcement_builder(announcement: &ServerAnnouncement) -> Result<EventBuilder, serde_json::Error> {
    let content = serde_json::to_string(announcement)?;
    Ok(EventBuilder::new(Kind::Custom(NOSTR_KIND_SERVER_ANNOUNCEMENT.into()), content)
        .tag(Tag::identifier("server")))
}

pub async fn publish_server_announcement(client: Client, identity: ServerIdentity, announcement: ServerAnnouncement) -> Result<String, String> {
    let event = server_announcement_builder(&announcement)
        .map_err(|e| format!("serialize announcement: {e}"))?
        .sign_with_keys(&identity.keys)
        .map_err(|e| format!("sign announcement: {e}"))?;
    let output = client.send_event(&event).await
        .map_err(|e| format!("publish announcement: {e}"))?;
    Ok(event.id.to_string())
}
```

#### 4. Export announcement APIs
**File**: `crates/nostr_client/src/lib.rs`  
**Action**: modify

```rust
pub mod announcement;
pub use announcement::{
    ServerAnnouncement, NOSTR_KIND_SERVER_ANNOUNCEMENT, SERVER_ANNOUNCEMENT_VERSION,
};
pub use identity::{decode_nsec_or_ncryptsec, load_server_identity_from_env_or_file, ServerIdentity};
```

#### 5. Server announcement system
**File**: `crates/server/src/nostr_announcement.rs`  
**Action**: create

Publish on `OnEnter(AppState::Ready)` using `RelayPool::client`.

```rust
use bevy::prelude::*;
use nostr_client::{
    publish_server_announcement, RelayPool, ServerAnnouncement, ServerIdentity,
    SERVER_ANNOUNCEMENT_VERSION,
};
use server_lightyear::ServerNetworkConfig;

pub struct ServerAnnouncementPlugin;

impl Plugin for ServerAnnouncementPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(protocol::AppState::Ready), publish_announcement_on_ready);
    }
}

fn publish_announcement_on_ready(
    pool: Res<RelayPool>,
    identity: Res<ServerIdentity>,
    network: Res<ServerNetworkConfig>,
) {
    let client = pool.client.clone();
    let identity = identity.clone();
    let announcement = ServerAnnouncement {
        server_addr: std::net::SocketAddr::from((network.bind_addr, network.port)),
        cert_digest: load_cert_digest(),
        display_name: "Untitled Brawler Server".to_string(),
        version: SERVER_ANNOUNCEMENT_VERSION,
    };

    IoTaskPool::get().spawn(async move {
        match nostr_client::announcement::publish_server_announcement(client, identity, announcement).await {
            Ok(event_id) => info!(%event_id, "published Nostr server announcement"),
            Err(error) => panic!("failed to publish Nostr server announcement: {error}"),
        }
    }).detach();
}

fn load_cert_digest() -> String {
    include_str!("../../../certificates/digest.txt").trim().to_string()
}
```

#### 6. Register server announcement module for lib/tests
**File**: `crates/server/src/lib.rs`  
**Action**: modify

```rust
pub mod nostr_announcement;
```

#### 7. Server startup identity loading and plugin registration
**File**: `crates/server/src/main.rs`  
**Action**: modify

Load identity before `App::new()` plugin build, insert it as a resource, and add announcement plugin.

```rust
pub mod nostr_announcement;

use nostr_client::load_server_identity_from_env_or_file;
use nostr_announcement::ServerAnnouncementPlugin;

fn main() {
    let network_config = ServerNetworkConfig {
        cert_pem_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../certificates/cert.pem").into(),
        key_pem_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../certificates/key.pem").into(),
        ..Default::default()
    };
    let server_identity = load_server_identity_from_env_or_file(network_config.nsec_file_path.as_deref())
        .expect("SERVER_NSEC or keys/server.nsec must contain a valid nsec/ncryptsec");

    App::new()
        // existing plugins...
        .insert_resource(server_identity)
        .add_plugins(ServerNetworkPlugin { config: network_config })
        .add_plugins(ServerAnnouncementPlugin)
        .run();
}
```

### Verification
#### Automated
- [x] `pgrep -af 'cargo (build|check|test|run)|wasm-pack test'` shows no other active build/test before running cargo commands.
- [x] `cargo check-all` passes.
- [x] `cargo test-native` passes.
- [x] `cargo test -p nostr_client announcement` passes.
- [x] `cargo test -p nostr_client identity` includes raw `nsec1...` and `ncryptsec1...` decode tests.

#### Manual
- [x] `SERVER_NSEC=nsec1... RUST_LOG=nostr_client=debug,server=debug cargo server-log` logs a published announcement event id.
- [x] `SERVER_NSEC=ncryptsec1... SERVER_NSEC_PASSPHRASE=... RUST_LOG=nostr_client=debug,server=debug cargo server-log` logs the same publish success path.
- [x] With neither `SERVER_NSEC` nor `keys/server.nsec`, `cargo server-log` panics at startup with a clear error mentioning both sources.
- [x] Use `nak` or `nostr-tool` to subscribe to kind `30078` for the server pubkey; event exists and `content` parses as JSON with `server_addr`, `cert_digest`, `display_name`, and `version`.

---

## Phase 4: Client server-list in MainMenu

### Changes

#### 1. Server-list resource and subscription system
**File**: `crates/nostr_client/src/announcement.rs`  
**Action**: modify

Add client-side entries and a receiver resource. Convert SDK `PublicKey` to displayable data inside this crate.

```rust
use std::time::Instant;
use async_channel::Receiver;
use nostr_sdk::{Event, Filter, PublicKey};

#[derive(Clone, Debug)]
pub struct ServerListEntry {
    pub pubkey: PublicKey,
    pub addr: SocketAddr,
    pub cert_digest: String,
    pub display_name: String,
    pub received_at: Instant,
}

#[derive(Resource, Default, Clone, Debug)]
pub struct ServerList {
    pub entries: Vec<ServerListEntry>,
}

#[derive(Resource)]
pub struct ServerAnnouncementRx(pub Receiver<ServerListEntry>);

#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct ServerAnnouncementSubscriptionStarted(pub bool);

pub fn parse_server_announcement_event(event: &Event) -> Result<ServerListEntry, String> {
    if event.kind != Kind::Custom(NOSTR_KIND_SERVER_ANNOUNCEMENT.into()) {
        return Err(format!("unexpected announcement kind {}", event.kind));
    }
    let announcement: ServerAnnouncement = serde_json::from_str(&event.content)
        .map_err(|e| format!("invalid announcement JSON: {e}"))?;
    if announcement.version != SERVER_ANNOUNCEMENT_VERSION {
        return Err(format!("unsupported announcement version {}", announcement.version));
    }
    Ok(ServerListEntry {
        pubkey: event.pubkey,
        addr: announcement.server_addr,
        cert_digest: announcement.cert_digest,
        display_name: announcement.display_name,
        received_at: Instant::now(),
    })
}

pub fn poll_server_announcements(mut list: ResMut<ServerList>, rx: Option<Res<ServerAnnouncementRx>>) {
    let Some(rx) = rx else {
        trace!("poll_server_announcements: subscription receiver not ready");
        return;
    };
    while let Ok(entry) = rx.0.try_recv() {
        if let Some(existing) = list.entries.iter_mut().find(|existing| existing.pubkey == entry.pubkey) {
            *existing = entry;
        } else {
            list.entries.push(entry);
        }
    }
}
```

Subscription startup:

```rust
pub fn spawn_server_announcement_subscription(
    mut commands: Commands,
    mut started: ResMut<ServerAnnouncementSubscriptionStarted>,
    pool: Option<Res<RelayPool>>,
) {
    if started.0 {
        trace!("spawn_server_announcement_subscription: already started");
        return;
    }
    let Some(pool) = pool else {
        trace!("spawn_server_announcement_subscription: RelayPool not ready yet");
        return;
    };
    started.0 = true;
    let (tx, rx) = async_channel::unbounded();
    commands.insert_resource(ServerAnnouncementRx(rx));
    let client = pool.client.clone();
    IoTaskPool::get().spawn(async move {
        let filter = Filter::new().kind(Kind::Custom(NOSTR_KIND_SERVER_ANNOUNCEMENT.into()));
        let mut stream = client.stream_events(filter, std::time::Duration::from_secs(60)).await
            .expect("server announcement stream must start");
        while let Some(event) = stream.next().await {
            match parse_server_announcement_event(&event) {
                Ok(entry) => { let _ = tx.send(entry).await; }
                Err(error) => warn!(%error, "ignored invalid server announcement"),
            }
        }
    }).detach();
}
```

Use exact stream type import from `futures-lite::StreamExt`.

#### 2. Plugin registration for discovery
**File**: `crates/nostr_client/src/plugin.rs`  
**Action**: modify

```rust
use crate::announcement::{poll_server_announcements, spawn_server_announcement_subscription, ServerAnnouncementSubscriptionStarted, ServerList};

impl Plugin for NostrClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerList>()
            .init_resource::<ServerAnnouncementSubscriptionStarted>()
            // existing setup...
            .add_systems(Update, (spawn_server_announcement_subscription, poll_server_announcements));
    }
}
```

`spawn_server_announcement_subscription` must run once from `Update` when `RelayPool` first exists, then set `ServerAnnouncementSubscriptionStarted(true)`.

#### 3. UI marker for server entries
**File**: `crates/ui/src/components.rs`  
**Action**: modify

```rust
#[derive(Component)]
pub struct ServerListEntryButton(pub nostr_client::announcement::ServerListEntry);
```

Store the entry clone directly so click handling does not need to re-index into a changed list.

#### 4. Main menu server list and selection
**File**: `crates/ui/src/lib.rs`  
**Action**: modify

`setup_main_menu` reads `ServerList` and spawns one button per entry above the fallback/manual Connect button.

```rust
fn setup_main_menu(mut commands: Commands, server_list: Res<nostr_client::announcement::ServerList>) {
    // existing root/title
    for entry in &server_list.entries {
        let label = format!("{}\n{}\n{}", entry.display_name, entry.addr, entry.pubkey);
        widgets::spawn_button(parent, &label, ServerListEntryButton(entry.clone()));
    }
    // keep existing Connect and Quit buttons
}
```

Update interaction handling:

```rust
fn main_menu_button_interaction(
    mut next_state: ResMut<NextState<ClientState>>,
    mut exit_writer: MessageWriter<AppExit>,
    mut config: ResMut<UiClientConfig>,
    identity: Option<Res<nostr_client::ClientIdentity>>,
    entry_query: Query<(&Interaction, &ServerListEntryButton), Changed<Interaction>>,
    connect_query: Query<&Interaction, (Changed<Interaction>, With<ConnectButton>)>,
    quit_query: Query<&Interaction, (Changed<Interaction>, With<QuitButton>)>,
) {
    for (interaction, entry) in &entry_query {
        if *interaction == Interaction::Pressed {
            let identity = identity.as_ref().expect("ClientIdentity must exist before server selection");
            config.server_addr = entry.0.addr;
            config.certificate_digest = entry.0.cert_digest.clone();
            config.client_id = nostr_client::client_id_from_public_key(&identity.public);
            info!(pubkey=%identity.public, server=%entry.0.pubkey, addr=%entry.0.addr, "selected Nostr server");
            next_state.set(ClientState::Connecting);
        }
    }
    // existing Connect fallback and Quit handling
}
```

#### 5. Client network config behavior
**File**: `crates/client_lightyear/src/connection.rs`  
**Action**: modify

No structural change should be required because `UiClientConfig` already mutates `client_id`, `server_addr`, and `certificate_digest` before `on_entering_connecting_state` builds a fresh `NetcodeClient`. Add a doc-comment to `ClientNetworkConfig.client_id` clarifying startup value may be replaced by UI before connect.

```rust
/// Lightyear netcode client id. The UI may replace this with a value derived
/// from the logged-in Nostr public key before `Connect` is triggered.
pub client_id: u64,
```

### Verification
#### Automated
- [x] `pgrep -af 'cargo (build|check|test|run)|wasm-pack test'` shows no other active build/test before running cargo commands.
- [x] `cargo check-all` passes.
- [x] `cargo test-native` passes.
- [x] `cargo test -p nostr_client announcement` covers event JSON parse, version rejection, and replacement by pubkey.
- [x] `cargo test -p ui ui_plugin` passes after `ServerList` UI dependency changes.

#### Manual
- [x] Run `SERVER_NSEC=nsec1... RUST_LOG=nostr_client=debug cargo server-log`; wait for announcement publish.
- [x] Run `RUST_LOG=nostr_client=debug cargo client`; unlock/generate identity; after EOSE, `MainMenu` shows the server entry with display name, address, and signing pubkey.
- [x] Click the server entry; UI transitions to `Connecting`, and existing flow connects using pre-shared netcode key.
- [x] Kill server; client entry remains visible after returning to `MainMenu`.
- [x] Restart server with same `SERVER_NSEC` and changed address/port if configurable; entry is replaced rather than duplicated for that pubkey.

---

## Phase 5: Post-connect challenge + `PlayerIdentity`

### Changes

#### 1. Protocol auth module with SDK-free wire types
**File**: `crates/protocol/src/auth/mod.rs`  
**Action**: create

This is the approved deviation from structure.md's `nostr_sdk::Event` in protocol.

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
#[type_path = "protocol::auth"]
pub struct NostrPublicKey(pub [u8; 32]);

impl NostrPublicKey {
    pub fn client_id_prefix(self) -> u64 {
        u64::from_le_bytes(self.0[0..8].try_into().expect("NostrPublicKey has 32 bytes"))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Message)]
pub struct IdentityChallenge {
    pub nonce: [u8; 32],
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Message)]
pub struct IdentityProof {
    pub pubkey: NostrPublicKey,
    pub signed_event_json: String,
}

#[derive(Channel)]
pub struct AuthChannel;

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[type_path = "protocol::auth"]
pub struct PlayerIdentity(pub NostrPublicKey);
```

#### 2. Register auth channel/messages/components
**File**: `crates/protocol/src/lib.rs`  
**Action**: modify

```rust
pub mod auth;
pub use auth::{AuthChannel, IdentityChallenge, IdentityProof, NostrPublicKey, PlayerIdentity};

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.add_channel::<AuthChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        }).add_direction(NetworkDirection::Bidirectional);

        app.register_message::<IdentityChallenge>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<IdentityProof>()
            .add_direction(NetworkDirection::ClientToServer);

        app.register_component::<PlayerIdentity>();
        app.register_type::<NostrPublicKey>();
        // existing registrations unchanged
    }
}
```

Do not replicate `PlayerIdentity` unless a later reviewed phase requires it.

#### 3. Nostr auth conversion helpers
**File**: `crates/nostr_client/src/auth.rs`  
**Action**: create

Keep all `nostr_sdk` event parsing/signing in `nostr_client`; client/server call these helpers and do not add direct `nostr-sdk` dependencies.

```rust
use nostr_sdk::{Event, EventBuilder, JsonUtil, Kind};
use protocol::{IdentityProof, NostrPublicKey, PlayerIdentity};
use crate::ClientIdentity;

pub const NOSTR_KIND_AUTH: u16 = 22242;

pub fn build_identity_proof(identity: &ClientIdentity, nonce: [u8; 32]) -> Result<IdentityProof, String> {
    let keys = nostr_sdk::Keys::new(identity.secret.clone());
    let event = EventBuilder::new(Kind::Custom(NOSTR_KIND_AUTH.into()), "")
        .tag(nostr_sdk::Tag::custom("challenge", [hex::encode(nonce)]))
        .sign_with_keys(&keys)
        .map_err(|e| format!("sign identity proof: {e}"))?;
    Ok(IdentityProof {
        pubkey: NostrPublicKey(*identity.public.as_bytes()),
        signed_event_json: event.as_json(),
    })
}

pub fn verify_identity_proof(
    proof: &IdentityProof,
    expected_nonce: [u8; 32],
    expected_client_id: u64,
) -> Result<PlayerIdentity, String> {
    let event = Event::from_json(&proof.signed_event_json)
        .map_err(|e| format!("invalid Nostr event JSON: {e}"))?;
    if event.kind != Kind::Custom(NOSTR_KIND_AUTH.into()) {
        return Err(format!("identity proof event kind must be {NOSTR_KIND_AUTH}, got {}", event.kind));
    }
    if !event.verify_signature() {
        return Err("identity proof signature verification failed".to_string());
    }
    let event_pubkey = NostrPublicKey(*event.pubkey.as_bytes());
    if event_pubkey != proof.pubkey {
        return Err("identity proof pubkey does not match signed event pubkey".to_string());
    }
    if !event_has_nonce(&event, expected_nonce) {
        return Err("identity proof nonce tag mismatch".to_string());
    }
    if proof.pubkey.client_id_prefix() != expected_client_id {
        return Err(format!(
            "identity proof pubkey/client_id mismatch: proof={} remote={}",
            proof.pubkey.client_id_prefix(), expected_client_id
        ));
    }
    Ok(PlayerIdentity(proof.pubkey))
}

fn event_has_nonce(event: &Event, nonce: [u8; 32]) -> bool {
    let expected = hex::encode(nonce);
    event.tags.iter().any(|tag| tag.as_slice().get(0).map(|v| v.as_str()) == Some("challenge")
        && tag.as_slice().get(1).map(|v| v.as_str()) == Some(expected.as_str()))
}
```

**File**: `crates/nostr_client/src/lib.rs`  
**Action**: modify

```rust
pub mod auth;
pub use auth::{build_identity_proof, verify_identity_proof, NOSTR_KIND_AUTH};
```

#### 4. Server nonce and proof verification
**File**: `crates/server/src/auth.rs`  
**Action**: create

Use a component on connection entities so cleanup is automatic when the connection entity despawns; also observe `Disconnected` for explicit logs.

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use protocol::{IdentityProof, PlayerIdentity};
use std::time::Instant;

#[derive(Component, Debug)]
pub struct PendingAuth {
    pub nonce: [u8; 32],
    pub issued_at: Instant,
}

fn validate_identity_proof(
    proof: &IdentityProof,
    pending: &PendingAuth,
    remote_id: RemoteId,
) -> Result<PlayerIdentity, String> {
    nostr_client::verify_identity_proof(proof, pending.nonce, remote_id.0.to_bits())
}
```

Add systems:

```rust
pub fn cleanup_pending_auth_on_disconnect(
    trigger: On<Add, Disconnected>,
    pending: Query<(), With<PendingAuth>>,
) {
    if pending.get(trigger.entity).is_ok() {
        info!(client=?trigger.entity, "client disconnected during identity challenge");
    }
}
```

#### 5. Split connected handling and authenticated spawn
**File**: `crates/server/src/gameplay.rs`  
**Action**: modify

`handle_connected` no longer spawns a character. It sends a challenge on `AuthChannel` and inserts `PendingAuth` on the connection entity.

```rust
fn handle_connected(
    trigger: On<Add, Connected>,
    mut commands: Commands,
    mut challenge_senders: Query<&mut MessageSender<IdentityChallenge>>,
) {
    let client_entity = trigger.entity;
    let nonce = rand::random::<[u8; 32]>();
    commands.entity(client_entity).insert(crate::auth::PendingAuth {
        nonce,
        issued_at: std::time::Instant::now(),
    });
    challenge_senders
        .get_mut(client_entity)
        .expect("Client entity must have MessageSender<IdentityChallenge>")
        .send::<AuthChannel>(IdentityChallenge { nonce });
    info!(?client_entity, "sent identity challenge");
}
```

Move the existing character spawn body into a helper called only after proof validation:

```rust
#[allow(clippy::too_many_arguments)]
pub fn spawn_authenticated_character(
    commands: &mut Commands,
    client_entity: Entity,
    player_identity: PlayerIdentity,
    // existing queries/resources from old handle_connected
) {
    // Existing spawn code, preserving PlayerId(peer_id) for replicated legacy display if still needed.
    // Add PlayerIdentity to the connection entity before spawning:
    commands.entity(client_entity).insert(player_identity);
    // Spawn character and initial MapTransitionStart exactly as old handle_connected did.
}
```

Add a new `handle_identity_proof` system that drains `MessageReceiver<IdentityProof>`, validates, removes `PendingAuth`, inserts `PlayerIdentity`, and then calls the spawn helper. On validation failure, log the exact reason and trigger `Disconnect` for that client.

#### 6. Server auth plugin registration
**File**: `crates/server/src/lib.rs`  
**Action**: modify

```rust
pub mod auth;
```

**File**: `crates/server/src/gameplay.rs`  
**Action**: modify

Register proof handling in `ServerGameplayPlugin` because it owns connection lifecycle.

```rust
impl Plugin for ServerGameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(handle_connected);
        app.add_observer(crate::auth::cleanup_pending_auth_on_disconnect);
        app.add_systems(Update, crate::auth::handle_identity_proof);
        // existing systems
    }
}
```

#### 7. Client challenge signing
**File**: `crates/client/src/auth.rs`  
**Action**: create

Receive challenges and sign NIP-42-style auth events with the session `ClientIdentity`.

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use nostr_client::{build_identity_proof, ClientIdentity};
use protocol::{AuthChannel, IdentityChallenge, IdentityProof};

pub struct ClientAuthPlugin;

impl Plugin for ClientAuthPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, handle_identity_challenge.run_if(resource_exists::<ClientIdentity>));
    }
}

fn handle_identity_challenge(
    identity: Res<ClientIdentity>,
    mut receivers: Query<&mut MessageReceiver<IdentityChallenge>>,
    mut senders: Query<&mut MessageSender<IdentityProof>>,
) {
    for mut receiver in &mut receivers {
        for challenge in receiver.receive() {
            let proof = build_identity_proof(&identity, challenge.nonce)
                .expect("ClientIdentity should sign identity proof");
            for mut sender in &mut senders {
                sender.send::<AuthChannel>(proof.clone());
            }
        }
    }
}

```

If SDK `Tag::custom` signature differs, adjust method names only; preserve kind `22242`, challenge tag, signed JSON, and no `nostr-sdk` in `protocol`.

#### 8. Client auth module registration
**File**: `crates/client/src/lib.rs`  
**Action**: modify

```rust
pub mod auth;
```

**File**: `crates/client/src/main.rs`  
**Action**: modify (supporting wiring required by Phase 5)

```rust
use client::auth::ClientAuthPlugin;

// after ClientNetworkPlugin and before gameplay systems is fine
.add_plugins(ClientAuthPlugin)
```

**File**: `crates/web/src/main.rs`  
**Action**: modify (supporting wiring required by Phase 5)

```rust
use client::auth::ClientAuthPlugin;

.add_plugins(ClientAuthPlugin)
```

### Verification
#### Automated
- [x] `pgrep -af 'cargo (build|check|test|run)|wasm-pack test'` shows no other active build/test before running cargo commands.
- [x] `cargo check-all` passes.
- [x] `cargo test-native` passes.
- [x] `cargo test -p protocol` covers `NostrPublicKey::client_id_prefix`.
- [x] `cargo test -p server auth` covers valid proof, wrong nonce, wrong pubkey, invalid signature, and pubkey/client_id mismatch.
- [x] `cargo test -p client auth` covers client proof JSON includes kind `22242` and challenge tag.

#### Manual
- [ ] Run server and two clients with distinct generated identities; both pass login, connect, reach `InGame`, and have characters spawned only after proof success.
- [ ] Server log shows challenge sent, proof verified, and `PlayerIdentity(NostrPublicKey)` inserted before character spawn.
- [ ] Run native client with `-c 999` after login to force mismatch; server disconnects during challenge and logs pubkey/client_id mismatch.
- [ ] Kill a client after connect but before proof completes; server logs cleanup for a pending-auth disconnected entity.
- [ ] Confirm no character entity spawns for an unauthenticated or failed-auth connection.

---

## Phase 6: Map ownership keyed by `PublicKey`

### Changes

#### 1. Map ID owner type
**File**: `crates/protocol/src/map/types.rs`  
**Action**: modify

Use the SDK-free `NostrPublicKey` type approved above.

```rust
use crate::auth::NostrPublicKey;

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, Reflect)]
#[type_path = "protocol::map"]
#[require(ActiveCollisionHooks::FILTER_PAIRS)]
pub enum MapInstanceId {
    Overworld,
    Homebase { owner: NostrPublicKey },
}
```

Update tests:

```rust
const TEST_OWNER: NostrPublicKey = NostrPublicKey([42; 32]);
assert_ne!(MapInstanceId::Overworld, MapInstanceId::Homebase { owner: TEST_OWNER });
```

#### 2. Owner component
**File**: `crates/protocol/src/map/mod.rs`  
**Action**: modify

```rust
use crate::auth::NostrPublicKey;

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[type_path = "protocol::map"]
pub struct Owner(pub NostrPublicKey);

pub use types::{MapInstanceId, MapRegistry, MapSwitchTarget};
```

Also export `Owner` from `crates/protocol/src/lib.rs`:

```rust
pub use map::{/* existing */, Owner, /* existing */};
```

Register `Owner` in `ProtocolPlugin` without prediction/replication unless needed for server-side inspection:

```rust
app.register_component::<Owner>();
```

#### 3. Voxel map homebase marker
**File**: `crates/voxel_map_engine/src/instance.rs`  
**Action**: modify

Because `voxel_map_engine` should not depend on `protocol` or Nostr, remove the owner field if unused outside server. Ownership is represented by `MapInstanceId` and `protocol::map::Owner`.

```rust
/// Marker: this map is a player's homebase.
#[derive(Component)]
pub struct Homebase;
```

Update server construction from `Homebase { owner }` to `Homebase`.

#### 4. Server map ownership and switching
**File**: `crates/server/src/map.rs`  
**Action**: modify

`handle_map_switch_requests` reads `PlayerIdentity` from the connection entity instead of `RemoteId`.

```rust
pub fn handle_map_switch_requests(
    // existing params...
    player_identities: Query<&PlayerIdentity>,
    // remove remote_ids: Query<&RemoteId>,
) {
    // existing request loop...
    let identity = player_identities
        .get(client_entity)
        .expect("Authenticated client must have PlayerIdentity before map switch");
    let target_map_id = resolve_switch_target(&request.target, identity.0);
}

fn resolve_switch_target(target: &MapSwitchTarget, owner: NostrPublicKey) -> MapInstanceId {
    match target {
        MapSwitchTarget::Overworld => MapInstanceId::Overworld,
        MapSwitchTarget::Homebase => MapInstanceId::Homebase { owner },
    }
}
```

`init_overworld_entity` inserts owner using `ServerIdentity`:

```rust
fn init_overworld_entity(
    mut commands: Commands,
    save_path: Res<WorldSavePath>,
    server_identity: Res<nostr_client::ServerIdentity>,
    // existing params...
) {
    let owner = NostrPublicKey(*server_identity.keys.public_key().as_bytes());
    commands.spawn((
        // existing overworld components...
        protocol::map::Owner(owner),
        MapInstanceId::Overworld,
    ));
}
```

`spawn_homebase` signature and seed use `NostrPublicKey`:

```rust
fn spawn_homebase(
    commands: &mut Commands,
    owner: NostrPublicKey,
    // existing params...
) -> (Entity, MapTransitionParams) {
    let map_dir = Arc::new(map_save_dir(&save_path.0, map_id));
    let seed = load_homebase_seed(&map_dir, owner);
    // ...
    commands.spawn((
        instance,
        config,
        dimensions.clone(),
        Homebase,
        protocol::map::Owner(owner),
        Transform::default(),
        map_id.clone(),
        // stores...
    ));
}

fn load_homebase_seed(map_dir: &Arc<PathBuf>, owner: NostrPublicKey) -> u64 {
    match store.load(&()) {
        Ok(Some(meta)) => meta.seed,
        _ => seed_from_nostr_public_key(owner),
    }
}

fn seed_from_nostr_public_key(owner: NostrPublicKey) -> u64 {
    u64::from_le_bytes(owner.0[0..8].try_into().expect("NostrPublicKey has 32 bytes"))
}
```

#### 5. Save directory naming
**File**: `crates/server/src/persistence/mod.rs`  
**Action**: modify

Use stable lower-case hex for filesystem portability. This differs from structure's bech32 option but remains within "bech32 or hex".

```rust
pub fn map_save_dir(base: &Path, map_id: &MapInstanceId) -> PathBuf {
    match map_id {
        MapInstanceId::Overworld => base.join("overworld"),
        MapInstanceId::Homebase { owner } => base.join(format!("homebase_{}", hex::encode(owner.0))),
    }
}
```

Add `hex = "0.4"` to `crates/server/Cargo.toml` if not already available transitively; do not rely on transitive deps.

Update test assertion:

```rust
let owner = NostrPublicKey([0x2a; 32]);
assert_eq!(
    map_save_dir(base, &MapInstanceId::Homebase { owner }),
    PathBuf::from(format!("worlds/homebase_{}", hex::encode(owner.0)))
);
```

#### 6. Server gameplay map authorization
**File**: `crates/server/src/gameplay.rs`  
**Action**: modify

Ensure authenticated spawn inserts `PlayerIdentity` on the connection entity before any map-switch message can be handled, and remove any use of `RemoteId.to_bits()` as a durable owner. Keep `PlayerId(peer_id)` on the character only if still needed for existing replicated client behavior; do not use it for ownership.

```rust
commands.entity(client_entity).insert(player_identity);
// Character may still carry PlayerId(peer_id) for existing visuals/replication, but ownership is PlayerIdentity.
```

#### 7. Explicit line-target update
**File**: `crates/server/src/map.rs` around existing `handle_map_switch_requests`  
**Action**: modify

The current line near `crates/server/src/map.rs:1054` must no longer query `RemoteId` or call `remote_id.0.to_bits()`. It must query `PlayerIdentity` and pass `identity.0` to `resolve_switch_target`.

#### 8. Server persistence tests and compile fallout
**File**: `crates/server/src/persistence/mod.rs` and affected map tests  
**Action**: modify

Update all `MapInstanceId::Homebase { owner: 42 }` test fixtures to `NostrPublicKey([42; 32])`. Search for remaining `Homebase { owner: u64 }`, `homebase-`, and `RemoteId.to_bits()` usages and remove or update them in this phase.

### Verification
#### Automated
- [ ] `pgrep -af 'cargo (build|check|test|run)|wasm-pack test'` shows no other active build/test before running cargo commands.
- [ ] `cargo check-all` passes.
- [ ] `cargo test-native` passes.
- [ ] `cargo test -p protocol map_instance_id_equality` passes with `NostrPublicKey` owner.
- [ ] `cargo test -p server map_save_dir_homebase` passes with `homebase_<hex>` naming.
- [ ] `cargo test -p server map_transition` passes after map-switch authorization reads `PlayerIdentity`.
- [ ] Search verification: no remaining source references to `RemoteId.to_bits()` for map ownership and no `homebase-` save-dir format in server source/tests.

#### Manual
- [ ] Start server with `SERVER_NSEC`, start two clients with distinct identities, connect both, switch each to Homebase; directories under `worlds/homebase_<hex>/` are distinct.
- [ ] Restart server, restart client A with the same passphrase, switch to Homebase; previous save loads from the same `homebase_<hex>` directory.
- [ ] Restart client A with a different generated identity, switch to Homebase; a fresh distinct directory is created.
- [ ] Inspect or temporarily log the Overworld map entity's `Owner`; it matches `ServerIdentity::keys.public_key()` converted to `NostrPublicKey`.
- [ ] Pre-existing `worlds/homebase-<u64>` or `worlds/homebase_<u64>` directories remain untouched and are not migrated.

---

## Final cross-phase verification

#### Automated
- [ ] `pgrep -af 'cargo (build|check|test|run)|wasm-pack test'` shows no other active build/test before running final commands.
- [ ] `cargo check-all` passes.
- [ ] `cargo test-native` passes.
- [ ] `cargo web-build` passes.
- [ ] `cargo test -p nostr_client` passes.
- [ ] `cargo test -p protocol` passes.
- [ ] `cargo test -p client` passes.
- [ ] `cargo test -p server` passes.
- [ ] `cargo test -p ui` passes.

#### Manual
- [ ] Full flow: launch server with fresh `SERVER_NSEC`, launch two clients with different generated identities, verify Login → MainMenu → server-list entry → Connecting → InGame.
- [ ] Server announcement appears on relay as signed kind `30078`; client shows signing pubkey with address.
- [ ] Relay outage at startup keeps client in `Loading`; restoring at least one relay allows progress.
- [ ] Pubkey/client_id tampering disconnects during post-connect challenge before character spawn.
- [ ] Two identities map to two separate Homebase directories; same identity/passphrase returns to the same directory after restart.
- [ ] Overworld `Owner` is server pubkey; Homebase owner is validated client `PlayerIdentity` pubkey.
