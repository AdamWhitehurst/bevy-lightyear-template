# Research Questions

## Context

Focus on the existing map persistence, map lifecycle, transition, identity, and networking boundaries across `server`, `client`, `protocol`, `voxel_map_engine`, `persistence`, and `nostr_client`. Pay special attention to how map data classes are represented, saved, loaded, replicated, validated, and associated with identities or map instances.

## Questions

1. How does the current filesystem persistence flow handle map metadata, terrain chunks, per-chunk entity spawns, and map-level saved entities from startup/load through edit/save, including `None` versus authoritative-empty semantics?
2. How are map instances, owners, and map types represented across `protocol`, server map spawning, client map registries, and save directory layout, and where are overworld and homebase behavior already differentiated?
3. How does the server map lifecycle advance through `MapLoadState`, store-operation polling, chunk generation, entity spawning, and readiness today, and which systems assume a map is immediately usable once its entity exists?
4. How does the map transition flow process client switch requests, create or find target maps, relocate/freeze players, send transition messages, and wait for client readiness, including all places that require seed, dimensions, or chunk data before progress continues?
5. How are voxel edits and world-object edits validated, acknowledged, replicated, marked dirty, and persisted, and what guarantees exist that client-visible state reflects server-authoritative applied changes?
6. What identity and signing infrastructure already exists in `nostr_client` and `protocol` for client identities, server identities, auth proofs, relay pool readiness, event publication, subscription, filtering, and error handling?
7. What persisted entity kinds, world-object definitions, component reflection paths, allowlists, bounds checks, quotas, and schema/version checks currently exist for accepting or rejecting loaded map data?
8. How do chunk terrain files, chunk entity files, map metadata files, and map-level entity files encode versioning, dimensions, generation data, content completeness, and corruption/version-mismatch failures?
9. What tests currently cover map persistence, map transitions, filesystem fallback, identity ownership, voxel/world-object edits, and Nostr relay behavior, and what test utilities exist for simulating failures or alternate stores?
10. How are errors and degraded states surfaced today for persistence loads/saves, relay availability, identity readiness, and map transitions, and are there existing enums/resources/events suitable for stale, divergent, missing, or unavailable state?
11. How is `nostr_client` wired into server and client crates today, what optional features or dependency boundaries exist, and where would backend-agnostic persistence interfaces need to remain free of Nostr/Blossom types?
12. What local Blossom/Nostr source or documentation is present under `git/`, and what event kinds, Blossom endpoints, authorization events, server-list events, content hashes, and filtering patterns are relevant to the current crate versions?
