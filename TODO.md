# TODO

## Planning

- [ ] Persistence
  - [x] abstract persistence to separate systems
  - [x] Filesystem Backends
  - [ ] NosterBackend
    - [ ] nostr saving of maps using editable nostr event + blossom blob storage
    - [ ] nostr prefetching, caching of chunks into fs
      - NostrBackend queries nearby
- [ ] NostrClient for clients and server
  - [ ] manages keys
  - [ ] manages nostr relays
  - [ ] can read/post/update/delete events
  - [ ] login + ui
  - [ ] new keys + ui
  - [ ] server discovery + ui
  - [ ] map ownership tied to nostr identities. Overworld = server's, Homebase = client's
- [ ] Nostr Node/Relay? May not be needed if can just use publicly available ones

- [ ] Singleplayer
  - [ ] Persistence

---


## Considering

- process distance sprite rigs at half the rate for performance?
- Social system that hooks into dialogue system
- Expose, split bevy-lightyear-template:
  - readme: how to add assets: animations, world objects, etc.
  - readme: built-in claude qrspi skills
  - stand-alone modules
- agent skills: component design, system design, networking, physics knowledge
- dampen player stats by distance from spawn?
- update world object system on hot reload by first remove WorldObjectId's components using old loaded Def, load new Def, insert_if_new(...) ?
- client --autoconnect flag
- stream ron assets to web clients on request
- bevi-inspector-egui
- wave function collapse
- Extend voxel_map_engine to support inserting pre-authored chunks
- composable Character templates that are loadable Asset files. Character template asset files are composed of other ron asset files
- monocraft ui font
- Ability unlock system
- Levelling system
- Singleplayer
- Stats and Buffs
- Npc interaction authoring via asset file writing
- NPCs
- Inventory/Item system
- animation creation still
- animation editor ui
- world object editor ui
- map editor ui
- spinning polyhedral exploding VFZ for ground_pound
