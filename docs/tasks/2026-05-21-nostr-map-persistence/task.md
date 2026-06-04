# Nostr Map Persistence

Implement v1 map/layout persistence that can use both local filesystem saves and Nostr/Blossom-backed portable saves. The goal is to preserve server-authoritative gameplay while allowing player-owned homebase map state and server-owned overworld state to survive restarts, backend outages, and cross-session recovery.
