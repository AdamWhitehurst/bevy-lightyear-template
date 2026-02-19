---
date: 2026-02-16T20:13:46-08:00
researcher: Claude
git_commit: c85072f4d39eba041b532a1e5a650c20066fbc9b
branch: master
repository: bevy-lightyear-template
topic: "Health system with respawn and billboard UI"
tags: [research, health, respawn, billboard-ui, hit-detection, replication]
status: complete
last_updated: 2026-02-16
last_updated_by: Claude
last_updated_note: "Resolved open questions with user decisions"
---

# Research: Health System with Respawn and Billboard UI

**Date**: 2026-02-16 20:13:46 PST
**Researcher**: Claude
**Git Commit**: c85072f4d39eba041b532a1e5a650c20066fbc9b
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question
How to implement a Player `Health` system that respawns players at a specified `RespawnPoint` component when health reaches 0, with health displayed as a billboard UI element above the player.

## Summary

The codebase currently has **no health, damage, death, or respawn systems**. Hit detection exists but only applies knockback. There is **no billboard or world-space UI** — all UI is screen-space menus. The implementation requires three new subsystems: (1) a server-authoritative `Health` component replicated without prediction, (2) server-side death detection + respawn at `RespawnPoint` entities, and (3) a client-side billboard health bar using child mesh quads that face the camera.

## Detailed Findings

### Current Player Entity Structure

Players spawn server-side in `handle_connected` when a client connects ([gameplay.rs:62-108](crates/server/src/gameplay.rs#L62-L108)):

| Component | Purpose |
|---|---|
| `Position(Vec3::new(x, 30.0, z))` | Spawn position (high Y, drops onto terrain) |
| `Rotation` | Facing direction |
| `ActionState<PlayerActions>` | Input state |
| `Replicate::to_clients(NetworkTarget::All)` | Network replication |
| `PredictionTarget::to_clients(NetworkTarget::All)` | Client prediction |
| `ControlledBy { owner: client_entity }` | Ownership binding |
| `CharacterPhysicsBundle` | Capsule collider, dynamic rigidbody, locked rotation |
| `ColorComponent` | Player color |
| `CharacterMarker` | Player entity marker |
| `AbilitySlots` | 4 ability slots (punch, dash, fireball, empty) |
| `AbilityCooldowns` | Cooldown tracking |

Client-side, predicted entities receive `CharacterPhysicsBundle` ([gameplay.rs:47-52](crates/client/src/gameplay.rs#L47-L52)) and rendering adds `Mesh3d` + `MeshMaterial3d` ([lib.rs:73-96](crates/render/src/lib.rs#L73-L96)).

A `DummyTarget` entity spawns at startup with the same physics but a `DummyTarget` marker ([gameplay.rs:20-33](crates/server/src/gameplay.rs#L20-L33)).

### Current Hit Detection (Knockback Only)

Hit detection runs in `FixedUpdate` after ability dispatch ([lib.rs:215-225](crates/protocol/src/lib.rs#L215-L225)):

**Melee** ([hit_detection.rs:63-104](crates/protocol/src/hit_detection.rs#L63-L104)): Spatial query with cuboid shape (0.75×1.0×0.5 half-extents) offset 1.5 units in facing direction. Tracks already-hit targets in `MeleeHitTargets` to prevent double-hits per active window.

**Projectile** ([hit_detection.rs:107-128](crates/protocol/src/hit_detection.rs#L107-L128)): Uses `CollidingEntities` sensor on bullet entities. Despawns bullet on first hit. Skips shooter via `ProjectileOwner`.

**Knockback** ([hit_detection.rs:130-146](crates/protocol/src/hit_detection.rs#L130-L146)): Horizontal direction from source to target + 0.3 upward component, normalized, scaled by force. Additive to `LinearVelocity`.

Both melee and projectile hit handlers currently only call `apply_knockback()`. These are the natural insertion points for damage application.

### Lightyear Replication Patterns

The project's [ProtocolPlugin](crates/protocol/src/lib.rs#L97-L161) registers components in two tiers:

**Predicted (rollback-enabled):**
- `Position`, `Rotation` — with custom threshold rollback + linear correction + interpolation
- `LinearVelocity`, `AngularVelocity` — with threshold rollback
- `ActiveAbility`, `AbilityCooldowns` — with default `PartialEq` rollback
- `ColorComponent`, `CharacterMarker`, `DummyTarget` — default rollback

**Replicated-only (no prediction):**
- `ChunkRenderTarget<MapWorld>`, `Name`, `AbilitySlots`, `AbilityProjectileSpawn`

Health should be **replicated-only** (no prediction). Reasoning from lightyear's design:
- Health changes are not deterministically derivable from local inputs
- Mispredicted health (flickering bar) is worse than ~100ms delay
- The lightyear spaceships demo uses this pattern for `Score` (analogous to Health)

### Current UI System

All UI uses screen-space Bevy `Node`-based system in [crates/ui/](crates/ui/src/lib.rs):
- `ClientState` enum: `MainMenu`, `Connecting`, `InGame`
- `DespawnOnExit` for state-driven lifecycle
- Standard `Button` + `Node` + `Text` patterns
- Single `Camera3d` with offset `(0, 9, -18)` following the controlled player

**No world-space UI, billboard components, or health bars exist.**

### Billboard UI Approaches for Bevy 0.17

Bevy 0.17 has **no built-in billboard component**. World-space UI is an [open tracking issue](https://github.com/bevyengine/bevy/issues/5476). Third-party crates (`bevy_health_bar3d`, `bevy_mod_billboard`) don't support 0.17 yet.

**Recommended: Custom billboard with child mesh quads**

Spawn a background quad + foreground quad as children of each character entity, offset above their head. A per-frame system rotates billboard entities to face the camera via `look_at`. Scale the foreground quad's X axis based on `current_hp / max_hp`.

```rust
#[derive(Component)]
struct Billboard;

fn billboard_face_camera(
    camera_q: Query<&GlobalTransform, With<Camera3d>>,
    mut billboard_q: Query<&mut Transform, With<Billboard>>,
) {
    let Ok(camera_gt) = camera_q.single() else { return };
    let camera_pos = camera_gt.translation();
    for mut transform in &mut billboard_q {
        transform.look_at(camera_pos, Vec3::Y);
    }
}
```

**Alternative: Screen-space overlay via `Camera::world_to_viewport()`** — projects 3D position to 2D screen coords and positions a Bevy UI `Node`. Pros: standard UI features. Cons: doesn't scale with distance, fragile with off-screen handling.

### Stats Design Context

From [doc/scratch/stats.md](doc/scratch/stats.md), the game's planned stat system includes **Vitality** as a primary stat affecting:
- Max health
- Injury resistance
- Survival chance in lethal encounters
- Aging rate

**Derived stat "Survivability"** = Vitality + Stamina, representing staying power. **Stamina** affects resistance to knockback. These design notes suggest health will eventually be derived from Vitality, though the initial implementation can use a simple flat value.

### RespawnPoint Pattern

No `RespawnPoint` components exist. Implementation would be a simple marker component placed on entities in the world (or spawned by the server at fixed positions). On death, the server queries for `RespawnPoint` entities and teleports the player.

## Architecture for Implementation

### Component Design

```
Health { current: f32, max: f32 }     — Replicated-only, server-authoritative
RespawnPoint { position: Vec3 }       — Server-only, not replicated
HealthBar (marker)                    — Client-only, child of character entity
Billboard (marker)                    — Client-only, for camera-facing rotation
```

### Data Flow

```
Server:  Hit Detection → apply_damage(Health) → lightyear replicates Health
Server:  Health ≤ 0 → query RespawnPoint → reset Position + Health
Client:  Receives replicated Health → update_health_bars scales bar mesh
Client:  Billboard system rotates health bar entities to face camera each frame
```

### System Insertion Points

| System | Schedule | Where |
|---|---|---|
| `apply_damage` | `FixedUpdate`, after `process_melee_hits` / `process_projectile_hits` | Shared protocol or server-only |
| `check_death_and_respawn` | `FixedUpdate`, after `apply_damage` | Server-only |
| `spawn_health_bar` | `Update`, on `Added<Predicted>` or `Added<Interpolated>` with `Health` | Client render crate |
| `update_health_bar` | `Update` | Client render crate |
| `billboard_face_camera` | `Update` or `PostUpdate` | Client render crate |

### Modification Points in Existing Code

1. **Hit detection** ([hit_detection.rs:63-146](crates/protocol/src/hit_detection.rs#L63-L146)): Add `&mut Health` to queries in `process_melee_hits` and `process_projectile_hits`, subtract damage alongside knockback.

2. **Protocol registration** ([lib.rs:125-160](crates/protocol/src/lib.rs#L125-L160)): Add `app.register_component::<Health>();` (replicate-only).

3. **Server spawn** ([gameplay.rs:85-107](crates/server/src/gameplay.rs#L85-L107)): Add `Health { current: 100.0, max: 100.0 }` to character spawn bundle.

4. **Client render** ([render/lib.rs:73-96](crates/render/src/lib.rs#L73-L96)): Add health bar child entities when spawning character visuals.

5. **FixedUpdate chain** ([lib.rs:215-225](crates/protocol/src/lib.rs#L215-L225)): Add death/respawn system after hit detection.

## Code References

- [crates/protocol/src/lib.rs:97-161](crates/protocol/src/lib.rs#L97-L161) — ProtocolPlugin, component registration
- [crates/protocol/src/lib.rs:202-231](crates/protocol/src/lib.rs#L202-L231) — SharedGameplayPlugin system registration
- [crates/protocol/src/hit_detection.rs:63-104](crates/protocol/src/hit_detection.rs#L63-L104) — Melee hit processing
- [crates/protocol/src/hit_detection.rs:107-128](crates/protocol/src/hit_detection.rs#L107-L128) — Projectile hit processing
- [crates/protocol/src/hit_detection.rs:130-146](crates/protocol/src/hit_detection.rs#L130-L146) — Knockback application
- [crates/server/src/gameplay.rs:62-108](crates/server/src/gameplay.rs#L62-L108) — Server player spawn
- [crates/server/src/gameplay.rs:20-33](crates/server/src/gameplay.rs#L20-L33) — Dummy target spawn
- [crates/client/src/gameplay.rs:16-53](crates/client/src/gameplay.rs#L16-L53) — Client character setup
- [crates/render/src/lib.rs:73-96](crates/render/src/lib.rs#L73-L96) — Character visual spawning
- [crates/render/src/lib.rs:98-112](crates/render/src/lib.rs#L98-L112) — Camera follow system
- [crates/ui/src/lib.rs](crates/ui/src/lib.rs) — Existing screen-space UI
- [crates/protocol/src/ability.rs:36-52](crates/protocol/src/ability.rs#L36-L52) — Ability effect types (knockback_force values)

## Historical Context (from doc/)

- [doc/scratch/stats.md](doc/scratch/stats.md) — Vitality stat affects max health, survivability = Vitality + Stamina
- [doc/scratch/moba-stats.md](doc/scratch/moba-stats.md) — Alternative MOBA stat reference with HP as primary stat
- [doc/research/2026-02-13-hit-detection-system.md](doc/research/2026-02-13-hit-detection-system.md) — Hit detection research (current system)
- [doc/plans/2026-02-14-hit-detection-knockback.md](doc/plans/2026-02-14-hit-detection-knockback.md) — Hit detection + knockback implementation plan
- [doc/design/2026-02-13-ability-effect-primitives.md](doc/design/2026-02-13-ability-effect-primitives.md) — Ability effect primitives design

## External References

- [Bevy world-space UI tracking issue #5476](https://github.com/bevyengine/bevy/issues/5476)
- [bevy_health_bar3d](https://github.com/sparten11740/bevy_health_bar3d) — Billboard health bars, supports up to Bevy 0.16
- [Lightyear spaceships demo protocol](https://github.com/cBournhonesque/lightyear/blob/main/demos/spaceships/src/protocol.rs) — Score component pattern (replicate-only)

## Resolved Design Decisions

1. **Damage values**: Abilities define a `base_damage` field in their RON definitions (alongside `knockback_force`).
2. **Invulnerability frames**: Yes — brief invulnerability period after respawn.
3. **Death animation/delay**: Instant teleport to respawn point, no delay.
4. **Health bar visibility**: Configurable, default to showing on all characters.
5. **Vitality integration**: Flat value initially (e.g., 100 HP). Vitality stat integration deferred.
6. **DummyTarget health**: Yes, DummyTarget gets health and respawns.
