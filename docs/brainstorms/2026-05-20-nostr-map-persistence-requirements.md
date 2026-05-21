---
date: 2026-05-20
topic: nostr-map-persistence
---

# Nostr Map Persistence

## Problem Frame

Map edits need to survive beyond a single local filesystem and respect map ownership. The overworld is server-owned, while each homebase is player-owned. Persistence should add Nostr-backed shared state for portability while preserving filesystem saves; load selection should use the newest valid copy rather than fixed backend priority.

This is v1 **map/layout persistence**, not full Home-Base simulation persistence. It supports the vision's Home-Base / Overworld split: players own and customize homebase map layout, while the server authoritatively hosts and saves loaded maps plus progression-bearing state.

---

## Actors

- A1. Server: Owns and edits the overworld; authoritatively hosts gameplay, replication, validation, and filesystem saves for loaded maps.
- A2. Player client: Owns the homebase identity and signs/publishes portable homebase map snapshots to Nostr/Blossom, but does not authoritatively maintain the runtime map or filesystem cache.
- A3. Filesystem store: Server-side durability/cache for map data.
- A4. Nostr relay network: Shared persistence transport for signed map data.

---

## Key Flows

- F1. Overworld save/load
  - **Trigger:** Server starts, edits overworld chunks/entities, or shuts down.
  - **Actors:** A1, A3, A4
  - **Steps:** Server loads the newest valid overworld copy from Nostr/filesystem, edits during play, then dual-writes saves or chunk updates to both stores.
  - **Outcome:** The shared overworld survives server restarts and can recover from either persistence backend.
  - **Covered by:** R1, R2, R3, R5, R6, R15, R16, R17

- F2. Homebase save/load
  - **Trigger:** Player enters, edits, leaves, or syncs their homebase.
  - **Actors:** A1, A2, A3, A4
  - **Steps:** The server authoritatively hosts, validates, applies edits to, and filesystem-saves the loaded homebase map. After server ack and authoritative replication, the owning client signs and publishes Nostr/Blossom map updates from its local authoritative replica; the server does not need to send a separate map blob solely for publication. On load/import, the server validates owner-signed Nostr/Blossom data before accepting it into authoritative runtime state.
  - **Outcome:** Player-owned homebase map data is portable and cryptographically attributable to the owner, while gameplay authority remains server-side.
  - **Covered by:** R1, R2, R4, R7, R8, R12, R13, R14, R16, R19

---

## Requirements

**Store behavior**

- R1. Map persistence must support both Nostr and filesystem stores for the same map data.
- R2. Saves must dual-write to Nostr and filesystem when both are available.
- R3. Loads must prefer the newest valid copy across Nostr and filesystem, not blindly prefer one backend.
- R4. If one backend is unavailable, loading and saving must continue through the available backend and surface degraded sync status without corrupting local state.
- R5. Filesystem fallback must preserve `None = missing` and `Some(empty) = authoritative empty` semantics for every persisted data class.

**Ownership and authority**

- R6. Overworld persistence is server-owned; overworld save events must be attributable to an accepted server identity.
- R7. Homebase Nostr/Blossom publication is player-owned; portable homebase map snapshot or chunk-update events must be signed and published by the owning client identity.
- R8. Each homebase must have a canonical owner public key established at creation/import; valid portable homebase snapshots and chunk updates must be signed by that key.
- R9. The server must reject or quarantine homebase save data that is not attributable to that homebase owner.
- R10. Valid signatures prove attribution, not game validity; owner-signed snapshots must still pass schema, bounds, size, entity allowlist, coordinate, and quota validation before loading into server-hosted runtime state.
- R11. Client-published homebase map snapshots must not mint progression-bearing objects, earned inventory, character state, relationships, breeding state, or rewards.
- R12. Coordination between server and client must make clear when server-hosted homebase state is older, equal to, newer than, or divergent from the owner's latest valid persisted snapshot or chunk-update chain.
- R13. The server remains authoritative for loaded homebase runtime state: it validates edit requests, applies edits, manages replication, and writes the server-side filesystem save. Client Nostr/Blossom publication is a portability/export channel, not a client-local filesystem authority.
- R14. Homebase clients may publish from their local map data only after server ack and authoritative replication of the applied change; clients must not publish predicted or unacknowledged edits. Publish-capable clients must have all persisted data classes needed for the published unit, including terrain chunks, per-chunk entity spawn data, map metadata, and map-level saved entities. The server may provide revision-chain metadata such as revision, previous hash, map id, and publish checkpoint, but need not send a separate blob of already-replicated map data solely for publication.

**Data coverage**

- R15. Persistence must cover terrain chunks, per-chunk entity spawn data, map metadata, and map-level saved entities required to restore a map coherently.
- R16. V1 Nostr/Blossom persistence must support chunk-level updates for all map types, not only whole-map snapshots. Manifests must be able to identify the latest valid per-chunk terrain/entity payloads plus required map metadata and map-level saved entities needed to reconstruct a complete map state.
- R17. Existing server filesystem saves must remain loadable as a fallback/migration source.
- R18. After migration, filesystem and Nostr saves should use the same attribution/validation envelope; unsigned legacy filesystem saves need explicit migration rules.

**User-visible behavior**

- R19. A player should be able to edit a homebase map on a server, have the owning client publish validated snapshots or chunk updates to Nostr/Blossom after server ack, and later recover the latest valid state on another server/session using the same Nostr identity.
- R20. A server restart should restore the latest valid overworld state from either Nostr or filesystem.
- R21. V1 sync status may be internal/log/debug-visible plus a simple system-readable enum; polished player-facing conflict UI is out of scope unless separately required.
- R22. Map transition requests that need remote persistence data must enter a server-owned transition preparing state before the player is relocated/frozen into the destination map. The server may delay starting the existing map transition while it fetches, validates, and selects Nostr/Blossom/filesystem data.
- R23. Nostr/Blossom-specific loading must stay in server or persistence integration code; `voxel_map_engine` must remain backend-agnostic and must not depend on `nostr_client`.
- R24. Server map lifecycle must gate transition progress on map readiness. `ensure_map_exists` may spawn the target map in a non-ready state such as `CheckingRemote`, `LoadingBlossom`, or `Reconciling`; chunk generation, chunk push, and transition completion must wait for `MapLoadState::Ready`.
- R25. If transition start messages require seed/dimensions before the map is ready, planning must define whether manifest/meta must be fetched before `MapTransitionStart` is sent or whether seed/dimensions are stable independently of remote save data.

---

## Acceptance Examples

- AE1. **Covers R2, R3.** Given filesystem has revision 5 and Nostr has revision 7 for a map, when the map loads, revision 7 is used.
- AE2. **Covers R3, R17.** Given Nostr has no valid save and filesystem has a valid existing save, when the map loads, the filesystem save is used.
- AE3. **Covers R7, R8, R9.** Given a homebase belongs to player P, when another identity publishes a save for P's homebase, the server ignores or quarantines it as non-authoritative.
- AE4. **Covers R4, R21.** Given Nostr is unreachable during a save, when the map is edited, the server-side filesystem save still completes and sync status indicates Nostr is behind/unavailable.
- AE5. **Covers R10, R11.** Given the owning player signs a save containing forbidden entities or excessive objects, when the server evaluates it, the snapshot is rejected or quarantined rather than loaded.
- AE6. **Covers R18.** Given a legacy unsigned filesystem homebase save exists, when migration runs, the system either binds it to the owning identity under explicit migration rules or treats it as server-local fallback until attribution is established.

---

## Success Criteria

- Homebase map persistence is portable across servers/sessions for the owning player identity while live map authority remains server-side.
- Overworld persistence remains server-authoritative and resilient to relay or filesystem failure.
- Planning can proceed with ownership, backend priority, and fallback semantics fixed at the requirement level; planning must still define revision, storage-shape, and sync mechanics.

---

## Scope Boundaries

- Do not implement full Home-Base simulation persistence in v1; character state, breeding, relationships, inventories, rewards, and autonomous behavior are separate systems.
- Do not implement general user-generated public map browsing in v1.
- Do not add multiplayer merge editing for the same homebase in v1; latest valid revision wins only within the agreed revision/conflict model.
- Do not replace filesystem persistence; it remains durable server-side cache/fallback.
- Do not make the server sign player-owned Nostr/Blossom homebase snapshots on behalf of clients.
- Do not add polished conflict-resolution UI in v1 unless explicitly chosen later.

---

## Key Decisions

- Dual-write and load newest valid copy: maximizes resilience while allowing Nostr portability.
- Client signs and publishes portable homebase map snapshots or chunk updates after server ack and authoritative replication: preserves player ownership and portability without making the client authoritative for live gameplay state.
- Server owns overworld saves: matches the existing shared world authority model.
- Server authoritatively hosts, validates, replicates, and filesystem-saves loaded homebases; client Nostr/Blossom publication is an export/import channel for owner-attributed portability. Publish-capable clients use their authoritative local replica and must have all persisted data classes for the published unit, so the server does not need to send a separate map blob solely for publication.
- Remote persistence fetches should be coordinated by a server-side transition preparing state before the existing map transition begins; this keeps players in their current map while save selection runs.
- Server map lifecycle should gate transition progress: `MapLoadState` can grow states such as `CheckingRemote`, `LoadingBlossom`, and `Reconciling`, and chunk generation, chunk push, and transition completion should wait for `Ready`.
- `MapTransitionStart` needs seed/dimensions, so planning must decide whether manifest/meta is fetched before sending it or whether seed/dimensions are stable independently.
- `voxel_map_engine` must not know about Nostr or Blossom. Backend-specific fetch/reconcile behavior belongs in server/persistence integration code, with the voxel engine consuming only selected, validated map data through backend-agnostic persistence boundaries.

---

## Dependencies / Assumptions

- Existing chunk terrain, per-chunk entity persistence, map metadata, and map-level entities are the first data classes to support.
- Save data uses a monotonic revision chain: each save records a revision and previous save hash so newest valid copy is deterministic and replay/rollback resistant.
- Nostr events need enough namespacing to avoid mixing overworld, homebase owners, chunks, entities, and unrelated apps.
- Relay and Blossom data should be treated as untrusted: valid signatures, content hashes, completeness checks, and schema validation are required before accepting save data.
- Blossom research source: `git/blossom` documents HTTP blob storage addressed by sha256, upload via `PUT /upload`, retrieval via `GET /<sha256>`, user server lists via Nostr kind `10063`, and optional Nostr authorization tokens via kind `24242`.
- Private signing keys must not be committed, embedded in save payloads, or logged.

---

## Outstanding Questions

### Resolve Before Planning

- [Resolved] V1 freshness uses a monotonic revision chain: each save records a revision and previous save hash; divergent or ambiguous chains are rejected/quarantined rather than silently overwritten.
- [Resolved] V1 Nostr map persistence uses manifest events plus chunk/entity payload references and supports chunk-level updates for all map types. Large payloads are stored in Blossom blobs addressed by sha256; Nostr events carry the signed revision-chain manifest, ownership metadata, content hashes, and Blossom retrieval hints.
- [Resolved] Transition-time remote loading uses a server-owned transition preparing state before player relocation/freezing. Nostr/Blossom integration stays outside `voxel_map_engine`; the voxel engine remains backend-agnostic.
- [Resolved] Map entity lifecycle uses readiness gating: `ensure_map_exists` may create a non-ready map, remote/filesystem reconciliation advances `MapLoadState`, and chunk generation, chunk push, and transition completion wait for `MapLoadState::Ready`.
- [Affects R4, R12, R21, R22, R24][User decision] What minimum sync states and stale/divergent-state actions should exist before gameplay proceeds?

### Deferred to Planning

- [Affects R6, R18][Technical] Define server key rotation, accepted server identities, and unsigned overworld migration rules.
- [Affects R7, R8, R9][Technical] Define owner-signature validation path and homebase owner binding storage.
- [Affects R10][Technical] Define exact schema, bounds, object allowlist, and quota validation.
- [Affects R15, R16][Technical] Define save completeness checks and content hashing.
- [Affects R21][Technical] Define sync-status representation and where it is surfaced.

---

## Next Steps

-> Resume `/ce-brainstorm` to resolve blocking questions before `/ce-plan`
