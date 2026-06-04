# Design Discussion

## Current State

The game already has semantic map identity: `MapInstanceId::Overworld` and `MapInstanceId::Homebase { owner: NostrPublicKey }`, with separate side-local `MapRegistry` mappings and an `Owner` component (`crates/protocol/src/map/types.rs:14`, `crates/protocol/src/map/mod.rs:23`). Filesystem saves are split into four data classes: map metadata, terrain chunks, per-chunk world-object spawns, and map-level saved entities (`crates/server/src/persistence/mod.rs:13`, `crates/voxel_map_engine/src/persistence/mod.rs:12`, `crates/voxel_map_engine/src/persistence/mod.rs:46`, `crates/server/src/persistence/fs_map_entities.rs:35`).

Overworld startup has an explicit server load lifecycle, but only for filesystem meta/entities: `AwaitingMeta -> AwaitingEntities -> Ready` (`crates/server/src/map.rs:120`, `crates/server/src/map.rs:190`, `crates/server/src/map.rs:619`). Homebase spawning currently loads seed/meta synchronously enough to produce transition parameters (`crates/server/src/map.rs:2157`). Transition start relocates/freezes the player and sends seed, generation version, bounds, chunk size, and readiness parameters immediately after `ensure_map_exists` (`crates/server/src/transition.rs:35`, `crates/server/src/transition.rs:75`).

Nostr identity already supports signed client auth proofs and server identities (`crates/nostr_client/src/auth.rs:8`, `crates/nostr_client/src/identity.rs:16`). Relay readiness is represented as a boolean startup gate (`crates/nostr_client/src/relay_pool.rs:88`, `crates/protocol/src/app_state.rs:59`). Blossom is documented locally, but no application-level Blossom implementation exists yet; Blossom blobs are sha256-addressed and use `PUT /upload`, `GET /<sha256>`, server-list kind `10063`, and authorization kind `24242` (`git/blossom/README.md:37`, `git/blossom/buds/03.md:7`, `git/blossom/buds/11.md:7`).

## Desired End State

Map/layout persistence can choose the newest valid save chain across local filesystem and Nostr/Blossom without weakening server-authoritative gameplay. Homebase portable saves are player-owned, while overworld saves are server-owned. Remote saves are accepted only after signature, ownership, hash, completeness, schema, bounds, quota, and allowlist validation.

Correctness means:

- valid newer remote map data can restore a map when local filesystem state is missing or older;
- valid filesystem state can be used when remote services are unavailable;
- invalid, incomplete, or divergent save chains are quarantined/blocked rather than silently overwritten;
- transition start only happens after the server has selected valid map metadata and parameters;
- runtime edits still flow through the authoritative server validation/ack/replication path.

## Patterns to Follow

- Keep map identity semantic and entity IDs side-local through `MapInstanceId` and `MapRegistry` (`crates/protocol/src/map/types.rs:14`).
- Keep persistence data classes explicit: meta, terrain chunks, chunk entities, map-level entities (`crates/server/src/persistence/mod.rs:13`, `crates/voxel_map_engine/src/persistence/mod.rs:12`, `crates/voxel_map_engine/src/persistence/mod.rs:46`).
- Preserve backend-agnostic store boundaries: the `Store<K, V>` trait returns `Ok(None)` only for absent values and errors for invalid data (`git/bevy-persistence/src/store.rs:35`, `git/bevy-persistence/src/store.rs:49`).
- Keep `voxel_map_engine` free of protocol/Nostr/Blossom types; it already uses string object IDs to avoid depending on `protocol` (`crates/voxel_map_engine/src/config.rs:40`).
- Follow existing server-authoritative edit flow: validate, apply, ack/broadcast, mark dirty, persist (`crates/server/src/map.rs:889`, `crates/server/src/map.rs:1615`, `crates/server/src/map.rs:1681`).
- Follow explicit startup/readiness gates with `trace!` early-outs, not silent `return`s (`crates/protocol/src/app_state.rs:59`, `crates/nostr_client/src/relay_pool.rs:88`).

Patterns not to extend:

- Do not preserve silent load-error fallback for remote persistence. Filesystem meta/entity load errors currently warn and continue/default (`crates/server/src/map.rs:218`, `crates/server/src/map.rs:619`), but remote invalidity/divergence must be explicit.
- Do not make map existence imply map usability. Current code has `expect` assumptions once a map entity is registered (`crates/server/src/map.rs:2124`, `crates/server/src/map.rs:834`); remote-backed loading needs a distinct preparing/not-ready lifecycle.

## Design Decisions

1. **Degradation policy**: graceful unavailable, strict invalid/divergent — if Nostr/Blossom is unreachable, use a valid filesystem save; if any available save chain is invalid, incomplete, or divergent, quarantine/block instead of choosing a potentially corrupt fork.
2. **Publication granularity**: chunk-update-first manifests — manifests describe revision-chain metadata and changed terrain/entity payload references, not only whole-map snapshots. Complete snapshot manifests are not required for v1 correctness.
3. **Transition timing**: preflight before transition start — the server resolves remote/filesystem freshness and validates map metadata before relocating/freezing the player or sending `MapTransitionStart`, because transition parameters include seed/dimensions.
4. **State surface**: server-only explicit states — extend server map/load persistence state with remote-checking, unavailable fallback, validating/reconciling, quarantined/divergent, and ready outcomes. Do not add client-visible degraded-state protocol in v1.
5. **Authority boundary**: server remains authoritative at runtime — clients may publish player-owned homebase portable updates only from authoritative replicated state after server ack; publication is an export/import channel, not client-local gameplay authority.
6. **Dependency boundary**: Nostr/Blossom transport belongs in `nostr_client`; game/server/client code owns ownership policy, validation, reconciliation, and lifecycle decisions through backend-agnostic persistence boundaries. `voxel_map_engine` must not depend on Nostr/Blossom.

## What We're NOT Doing

- Not persisting progression-bearing character state, inventory, relationships, breeding state, rewards, or full homebase simulation state in v1.
- Not allowing client-published homebase data to mint earned objects or bypass server validation.
- Not adding client-visible degraded persistence UI/protocol in v1 beyond existing transition behavior/logging.
- Not making Blossom or relays trusted storage; all remote payloads remain content-addressed and validated.
- Not rewriting the existing filesystem stores; they remain the local backend and fallback source.
- Not introducing Nostr/Blossom dependencies into `voxel_map_engine`.

## Open Risks

- Chunk-update-first chains need careful completeness checks so a map is not assembled from missing prior chunks/entities.
- Server-only blocked states may be hard to diagnose without enough logging/test hooks.
- Divergence policy may need later UX/admin tooling to inspect quarantined branches.
- Homebase client publication from authoritative replicas requires the client to have all payload classes for the published unit; missing local replicas could delay or skip publication.
- Relay/Blossom outages during preflight can delay map switching until fallback selection completes.
