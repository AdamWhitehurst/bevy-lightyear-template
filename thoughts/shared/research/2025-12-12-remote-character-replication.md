---
date: 2025-12-12T08:19:33-08:00
researcher: Claude
git_commit: 24a300667401cda4ce8ab2e3abdde7c022b7fa39
branch: master
repository: bevy-lightyear-template
topic: "Remote character movements not replicating"
tags: [research, lightyear, networking, replication, prediction, cargo-features]
status: complete
last_updated: 2025-12-12
last_updated_by: Claude
last_updated_note: "Updated with correct root cause after first fix didn't work"
---

# Research: Remote Character Movements Not Replicating

**Date**: 2025-12-12T08:19:33-08:00
**Researcher**: Claude
**Git Commit**: 24a300667401cda4ce8ab2e3abdde7c022b7fa39
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question
After implementing a level following the `git/lightyear/examples/avian_3d_character/` example, local characters respond properly to inputs, but remote character movements are not being replicated.

## Summary

**Root Cause: Client crate is missing the `prediction` feature for lightyear.**

Without `prediction`, the `receive_remote_player_input_messages` system is never added to the client. This system is gated behind `#[cfg(feature = "prediction")]` in `lightyear_inputs/src/client.rs:185`. Rebroadcasted inputs from other players are received but never processed.

## Detailed Findings

### Issue #1: Missing Cargo Features (Critical)

**Reference example** (`git/lightyear/examples/avian_3d_character/Cargo.toml:31-37`):
```toml
lightyear = { workspace = true, features = [
  "interpolation",
  "prediction",      # ← CRITICAL
  "replication",
  "leafwing",
  "avian3d",
] }
```

**Current implementation** (`crates/client/Cargo.toml:10`):
```toml
lightyear = { workspace = true, features = ["client", "netcode", "udp", "crossbeam", "webtransport", "leafwing"] }
# MISSING: prediction, replication, interpolation
```

| Feature | Reference Example | Your Client | Required |
|---------|-------------------|-------------|----------|
| `prediction` | ✓ | ✗ | **Yes - enables remote input processing** |
| `replication` | ✓ | ✗ | Yes |
| `interpolation` | ✓ | ✗ | Recommended |

**Why this breaks remote replication:**

In `git/lightyear/lightyear_inputs/src/client.rs:185-220`:
```rust
#[cfg(feature = "prediction")]
if self.config.rebroadcast_inputs {
    // This system processes inputs from other players
    app.add_systems(
        PreUpdate,
        receive_remote_player_input_messages::<S>
            .in_set(InputSystems::ReceiveInputMessages),
    );
}
```

Without `prediction` feature:
1. `receive_remote_player_input_messages` system is never added
2. Server rebroadcasts inputs but client doesn't process them
3. Remote predicted entities never receive `InputBuffer` or `ActionState`
4. Movement system query fails (requires `&ActionState<PlayerActions>`)

### Issue #2: Server Missing ActionState (Secondary)

The server should also spawn characters with `ActionState` for completeness. This was the first fix attempted but didn't resolve the issue alone.

**Fixed in** `crates/server/src/gameplay.rs:82`:
```rust
ActionState::<PlayerActions>::default(),
```

## Code References

- `crates/client/Cargo.toml:10` - Missing `prediction` feature
- `git/lightyear/lightyear_inputs/src/client.rs:185-220` - Feature-gated system
- `git/lightyear/examples/avian_3d_character/Cargo.toml:31-37` - Reference features
- `crates/server/src/gameplay.rs:79-93` - Character spawn (ActionState added)

## Fix Required

### `crates/client/Cargo.toml` line 10:
```toml
lightyear = { workspace = true, features = ["client", "netcode", "udp", "crossbeam", "webtransport", "leafwing", "prediction", "replication", "interpolation"] }
```

### `crates/server/Cargo.toml` line 16 (optional consistency):
```toml
lightyear = { workspace = true, features = ["server", "netcode", "udp", "webtransport", "websocket", "leafwing", "replication"] }
```

## Data Flow (After Fix)

1. Client A sends inputs to server
2. Server processes inputs, updates Position via physics
3. Server rebroadcasts inputs to Client B (via `rebroadcast_inputs: true`)
4. Client B receives input message
5. **`receive_remote_player_input_messages` runs** (NOW ENABLED)
6. Client B inserts `InputBuffer` + `ActionState` on remote predicted entity
7. Client B's movement system processes remote entity
8. Both clients see both characters moving

## Open Questions
None - root cause identified.
