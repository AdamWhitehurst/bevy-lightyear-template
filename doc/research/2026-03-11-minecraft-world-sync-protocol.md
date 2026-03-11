---
date: 2026-03-11T10:52:30-07:00
researcher: claude
git_commit: 366fd09789153cff6059de24f3cbec535d71505a
branch: master
repository: bevy-lightyear-template
topic: "Minecraft server-client world synchronization protocol"
tags: [research, minecraft, networking, voxel-sync, block-changes, chunk-protocol]
status: complete
last_updated: 2026-03-11
last_updated_by: claude
last_updated_note: "Resolved open questions into decisions"
---

# Research: Minecraft Server-Client World Synchronization Protocol

**Date**: 2026-03-11T10:52:30-07:00
**Researcher**: claude
**Git Commit**: 366fd09789153cff6059de24f3cbec535d71505a
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How Minecraft servers and clients synchronize world changes (voxel/block edits). How this compares to the current project's approach.

## Summary

Minecraft uses a server-authoritative model with client-side prediction for block changes. Chunks are sent as paletted, zlib-compressed sections. Block edits use a sequence-number system (since ~1.19) for client prediction reconciliation. The protocol has distinct packets for: full chunk transfer, single block updates, batched section updates, chunk lifecycle, and lighting. The current project uses lightyear messages for a similar flow but lacks client prediction, batched updates, and per-section granularity.

## Minecraft Protocol: Chunk Data Transfer

### Full Chunk Packet (`chunk_data_and_update_light`, ID 0x45)

Sends a full chunk column (all sections) + light data in one packet.

| Field | Type | Notes |
|---|---|---|
| Chunk X, Z | Int | Column coordinates |
| Heightmaps | NBT | WORLD_SURFACE, MOTION_BLOCKING |
| Data | Byte Array | All sections serialized sequentially |
| Block Entities | Array | NBT for tile entities |
| Light Data | Struct | Sky + block light masks and arrays |

**When sent:**
- On player join: all chunks within `view-distance`
- On player movement across chunk border: new chunks entering view radius
- Sent roughly in spiral order outward from player position

### Section Format (inside Data)

Each 16x16x16 section:

| Field | Type |
|---|---|
| Block Count | Short (non-air count) |
| Block States | Paletted Container (4096 entries) |
| Biomes | Paletted Container (64 entries, 4x4x4) |

Empty sections are still present (block count = 0).

### Paletted Container Encoding

| Bits Per Entry | Mode | When Used |
|---|---|---|
| 0 | Single-valued | Uniform section (all one block) — just 1 VarInt, no data array |
| 4-8 | Indirect | 2-256 distinct types — local palette + packed long[] |
| 15+ | Direct | >256 types — global IDs, no palette |

Entries packed into 64-bit longs, LSB first. Entries never span long boundaries. Formula: `entries_per_long = floor(64 / bpe)`.

### Protocol-Level Compression

No chunk-specific compression. All packets > threshold (default 256 bytes, configurable via `network-compression-threshold`) are **zlib-compressed** at the transport layer. Chunk packets always exceed this.

## Minecraft Protocol: Block Change Synchronization

### Block Breaking (Client → Server)

**Packet: Player Action (`player_action`, ID 0x41)**

| Field | Type | Notes |
|---|---|---|
| Status | VarInt | START_DESTROY (0), ABORT_DESTROY (1), FINISH_DESTROY (2) |
| Location | Position | Block coordinates |
| Face | Byte | 0-5 (Down/Up/N/S/W/E) |
| Sequence | VarInt | Block change sequence counter |

Survival: sends START then FINISH after break animation. Creative: only START (instant).

### Block Placement (Client → Server)

**Packet: Use Item On (`use_item_on`, ID 0x64)**

| Field | Type | Notes |
|---|---|---|
| Hand | VarInt | MAIN_HAND (0), OFF_HAND (1) |
| Block Hit | Position | Block face was clicked on |
| Face | Byte | Which face (0-5) |
| Cursor X/Y/Z | Float | Exact click position on face |
| Is Inside Block | Boolean | Player head inside a block |
| Sequence | VarInt | Block change sequence counter |

### Server Processing

1. Receives packet
2. Validates: game mode, block exists, player in range, item valid, target valid
3. Applies change to world state
4. Broadcasts to nearby clients

### Server → All Nearby Clients: Block Update (`block_update`, ID 0x09)

| Field | Type |
|---|---|
| Location | Position |
| Block State ID | VarInt |

### Server → Originating Client: Acknowledge Block Change (`block_changed_ack`, ID 0x05)

| Field | Type |
|---|---|
| Sequence | VarInt |

Tells the originator: "processed up to this sequence — replace predictions with authoritative state."

## Client-Side Prediction (Sequence System, ~1.19+)

1. Client maintains a monotonically incrementing sequence counter
2. Every block interaction increments counter and includes it in the serverbound packet
3. Client **optimistically applies** the change locally (block appears/disappears instantly)
4. Server processes, sends `block_changed_ack` with sequence number
5. Client replaces predicted state with server's authoritative state on ack

**On server rejection:** Server sends `block_update` with the original state (e.g., air where client placed a block) + `block_changed_ack`. Client rolls back the prediction.

**Ghost blocks** happen when this reconciliation fails under high latency or rapid placement.

## Multi-Block Updates (Batched)

### Packet: Section Blocks Update (`section_blocks_update`, ID 0x83)

Fired when 2+ blocks change in the same 16x16x16 section on the same tick.

| Field | Type | Notes |
|---|---|---|
| Section Position | Long | Packed: X (22 MSB, signed), Z (22 bits), Y (12 LSB, signed) |
| Blocks | Array of VarLong | Each packs state + local position |

**Entry encoding:**
```
packed = (blockStateId << 12) | (localX << 8) | (localZ << 4) | localY
```

More efficient than N individual `block_update` packets — section position sent once.

## Chunk Loading/Unloading Protocol

### Set Center Chunk (`set_center_chunk`, ID 0x93)

| Field | Type |
|---|---|
| Chunk X/Z | VarInt |

Sent on chunk border crossing. Client uses this + view distance to determine expected chunk set.

### Forget Level Chunk (`forget_level_chunk`, ID 0x38)

| Field | Type |
|---|---|
| Chunk X/Z | Int |

Client discards the chunk column entirely.

### Lifecycle:
1. Player joins/moves → server sends `set_center_chunk`
2. Server sends `chunk_data_and_update_light` for new chunks entering view
3. Server sends `forget_level_chunk` for chunks leaving view

## Light Updates

### Light Data Structure

| Field | Type |
|---|---|
| Sky/Block Light Masks | BitSet | Which sections have data |
| Empty Sky/Block Light Masks | BitSet | Which sections are all-zero |
| Sky/Block Light Arrays | Array of 2048-byte arrays | 4 bits/block nibble arrays |

### Standalone Update Light (`update_light`, ID 0x48)

Sent when lighting changes without chunk data changes (e.g., torch placed after initial load). Initial light comes bundled in the chunk packet.

## Comparison: Current Project vs Minecraft

### Current Project Approach

Based on [crates/server/src/map.rs](crates/server/src/map.rs) and [crates/protocol/src/map.rs](crates/protocol/src/map.rs):

| Aspect | Current Project | Minecraft |
|---|---|---|
| **Chunk transfer** | Not sent over network — client generates locally from shared seed | Full chunk sections sent as paletted, compressed data |
| **Block edit flow** | Client → `VoxelEditRequest` → Server validates → `VoxelEditBroadcast` to all | Client → `player_action`/`use_item_on` → Server validates → `block_update` to nearby |
| **Initial sync** | `VoxelStateSync` sends ALL modifications as flat `Vec<(IVec3, VoxelType)>` on connect | Sends full chunk data per-column as player enters view |
| **Client prediction** | None — client waits for server broadcast | Sequence-number system with optimistic local apply |
| **Batching** | None — each edit is a separate message | `section_blocks_update` batches 2+ changes per section per tick |
| **Chunk lifecycle** | Client generates chunks locally based on `ChunkTarget` distance | Server explicitly sends/forgets chunks via packets |
| **Compression** | lightyear handles transport compression | zlib at protocol level for all packets > threshold |
| **Scope** | Global broadcast (`NetworkTarget::All`) | Nearby clients only (chunk-level visibility) |

### Key Architectural Differences

**1. Who generates terrain:**
- Minecraft: server generates, sends to client
- Current project: both sides generate from shared seed, only diffs synced

**2. Modification sync granularity:**
- Minecraft: per-block with sequence-based prediction
- Current project: per-block without prediction, full-state sync on connect

**3. Visibility scope:**
- Minecraft: only clients with the chunk loaded receive updates for blocks in that chunk
- Current project: `NetworkTarget::All` broadcasts to every connected client regardless of distance (lightyear rooms partially address this — player entities are room-scoped, but voxel broadcasts are global)

**4. State representation:**
- Minecraft: full authoritative world state on server, palette-compressed sections
- Current project: procedural base + modifications overlay, flat `Vec` of all edits

### What Minecraft Does That the Project Doesn't

1. **Client prediction with rollback** — block changes feel instant; the sequence system cleanly handles rejection
2. **Batched section updates** — multiple changes in one section on one tick are combined into one packet
3. **Scoped broadcasting** — only clients with the relevant chunk loaded receive block updates
4. **Chunk-level data transfer** — server sends actual chunk data rather than relying on shared generation
5. **Incremental light sync** — separate light update packets

### What the Project Does Differently (and Why)

1. **Shared seed generation** — avoids sending chunk data over the network entirely. Bandwidth-efficient for deterministic terrain. Trade-off: can't support pre-authored or non-deterministic terrain without full chunk transfer.
2. **Full modification list on connect** — simpler than chunked initial sync. Works at current scale but doesn't scale to large edit counts.
3. **lightyear transport** — handles serialization, compression, and reliable delivery. The project doesn't need to implement its own packet framing or compression.

## Sources

- [Java Edition protocol/Packets — Minecraft Wiki](https://minecraft.wiki/w/Java_Edition_protocol/Packets)
- [Java Edition protocol/Chunk format — Minecraft Wiki](https://minecraft.wiki/w/Java_Edition_protocol/Chunk_format)
- [Protocol — wiki.vg (archived)](https://c4k3.github.io/wiki.vg/Protocol.html)
- [PaperMC Issue #3053 — Ghost blocks](https://github.com/PaperMC/Paper/issues/3053)

## Code References

- [crates/server/src/map.rs:346-388](crates/server/src/map.rs#L346-L388) — `handle_voxel_edit_requests`: receives edit, applies, broadcasts to all
- [crates/server/src/map.rs:391-403](crates/server/src/map.rs#L391-L403) — `send_initial_voxel_state`: sends full modification list on connect
- [crates/protocol/src/map.rs:84-101](crates/protocol/src/map.rs#L84-L101) — `VoxelEditRequest`, `VoxelEditBroadcast`, `VoxelStateSync` message types
- [crates/server/src/map.rs:182-185](crates/server/src/map.rs#L182-L185) — `VoxelModifications`: flat `Vec<(IVec3, VoxelType)>` of all edits

## Historical Context (from doc/)

- `doc/research/2026-01-17-voxel-world-save-load.md` — Documents the current modifications-only persistence model
- `doc/research/2026-03-09-minecraft-style-map-directory-saving.md` — Minecraft world save format research (directory structure, region files)

## Decisions

1. **Client prediction for block edits**: Yes. Use Minecraft's sequence-number approach — client optimistically applies changes, server acks with sequence number, client reconciles.

2. **Scoped voxel broadcasts**: Yes. Chunks are only relevant to the room they belong to. Use lightyear room-scoped messaging (`NetworkTarget::Room(room_entity)`) instead of `NetworkTarget::All`.

3. **Per-chunk initial sync with palette encoding**: Replace the full `Vec<(IVec3, VoxelType)>` sync with per-chunk data transfer using palette-based encoding (matching Minecraft's paletted container format). Client receives chunk data as it enters view, not all modifications at once.

4. **Batched updates (Minecraft-style)**: Accumulate block changes per chunk section per tick. If a section has 1 change, send a single block update. If a section has 2+ changes on the same tick, batch them into one `SectionBlocksUpdate` message containing all changes for that section. This matches Minecraft's `section_blocks_update` packet behavior.
