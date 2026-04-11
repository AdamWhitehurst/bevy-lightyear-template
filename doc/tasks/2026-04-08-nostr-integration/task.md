# Nostr Integration

Integrate nostr (via `nostr-sdk`) to replace the current hardcoded/filesystem-based approaches to three concerns: **server discovery** (clients find game servers via nostr events instead of hardcoded addresses), **identity** (players identified by nostr keypairs instead of bare u64 client IDs), and **persistence** (game state saved/loaded via nostr events instead of local filesystem). The current filesystem persistence layer should be abstracted behind a trait so that the nostr backend can be swapped in without rewriting consumers.

- abstract persistence into systems to support multiple backends: filesystem and nostr+blossom
- nostr client
  - identity: server and players need a nostr key
  - server discovery
  - persistence
- nostr caching of chunk
- map ownership tied to nostr identities. Overworld = server's, Homebase = client's
