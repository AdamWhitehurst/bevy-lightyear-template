# Research Questions

## Context
Focus on: the persistence layer (chunk saves, map metadata, entity serialization), the networking/connection setup (client config, server config, transport), and identity/authentication (client ID assignment, how the ID flows through the system). Also examine how the asset loading pipeline works and what abstraction boundaries already exist.

## Questions

1. Trace the full lifecycle of chunk persistence: how does a chunk go from modified in-memory to written to disk, and from disk back to loaded? What formats, paths, and concurrency patterns are involved? What assumptions does the code make about the storage backend (e.g., atomic rename, random access, directory listing)?

2. How does the map metadata persistence (`map.meta.bin`, `WorldSavePath`, `SavedEntity`) work end-to-end? How are entities marked for persistence, serialized, and restored? What is the relationship between map metadata and chunk persistence?

3. How does the client establish a connection to the server today? Trace from config construction through transport setup to the first authenticated packet. Where is the server address resolved, and where is the client identity (u64) injected into the connection handshake?

4. How does the client ID (`u64`) propagate through the system after connection? Where is it used for ownership checks, per-player state, homebase naming, and replication filtering? What would change if the ID type became a 32-byte public key instead of a u64?

5. What abstraction boundaries exist (or don't) between the persistence consumers (chunk systems, map systems, entity serialization) and the storage mechanism (filesystem)? Are there traits, indirection layers, or is `std::fs` called directly?

6. How does `nostr-sdk` work in a Rust async context — what runtime does it expect, how are subscriptions managed, and what does the event publish/query API look like? What nostr event kinds (NIP-01 kinds, custom kinds) and tag conventions would be relevant for storing binary blobs and structured game data?

7. How do the self-signed TLS certificates and the hardcoded `PROTOCOL_ID` / `PRIVATE_KEY` participate in connection auth today? If identity moved to nostr keypairs, which of these mechanisms would need to change vs. could remain as transport-level plumbing?

8. What are the latency and size characteristics of the current persistence operations (chunk reads/writes, metadata saves)? How large are typical chunk files, how often are they written, and are there any batching or throttling patterns?
