# Research Questions

## Context
Focus on the `crates/protocol` crate, specifically `src/character/movement.rs` (movement input application, which currently contains jump handling inline) and the `src/ability/` subtree (activation, lifecycle, effects, spawn, types, loading, plugin). Also relevant: how physics-related systems are scheduled in the Bevy app, how components are stored (sparse-set vs. table) across the codebase, and how inputs from `PlayerActions` are routed to gameplay logic in `crates/client/src/gameplay.rs` and `crates/server/src/gameplay.rs`.

## Questions
1. How is jumping currently implemented in `crates/protocol/src/character/movement.rs` — specifically, how is the jump input detected, how is ground contact verified inline, and how is the resulting impulse or velocity change applied to the character's physics body?

2. Trace the end-to-end flow of an ability from input to effect: how is an ability triggered (input binding or otherwise), what does `ability/activation.rs` do, how does `ability/lifecycle.rs` progress its state, and how are the per-ability side-effects in `ability/effects.rs` applied to the world?

3. What is the data schema for an ability definition — what file format is used (RON, etc.), what fields exist on an ability asset, and how are they loaded and instantiated at runtime (see `ability/loader.rs`, `loading.rs`, `spawn.rs`, and `types.rs`)?

4. What conventions does this codebase use for marker-style components — are any components explicitly declared with sparse-set storage today, and where are such markers added/removed in systems?

5. Are there any existing activation-time gates on abilities today (cooldowns, resource costs, state checks, targeting prerequisites) — and if so, where in the ability pipeline are those checks performed, and how are failure outcomes signaled?

6. What is the system ordering for physics-adjacent systems — where does movement input run in the Bevy schedule relative to velocity integration, collision, and any existing ground-related logic, and what scheduling primitives (sets, configs, `before`/`after`) are used?

7. How are character physics bodies queried for ground contact today — which avian APIs (shape-cast, ray-cast, collision events, contact pairs) are in use in the codebase, and where are those helper call sites?

8. How is the `PlayerActions` input enum defined, what variants exist beyond `Jump`, and how are individual actions routed from input to either movement code or ability activation (compare `crates/client/src/gameplay.rs` and `crates/server/src/gameplay.rs`)?

9. How do abilities modify character velocity or apply impulses — do ability effects reach into the same physics forces/velocity components that `movement.rs` writes, and is there any existing contention or ordering required between the two?

10. How does the ability system integrate with networking (lightyear replication, client prediction, rollback) — what must a component or system register to be network-safe, and what patterns exist for predicted vs. confirmed ability effects?

11. What would it take to have the `ApplyForce` `AbilityEffect` be applied with `forces.apply_linear_impulse` (the same API `movement.rs` uses for jump) instead of setting velocity directly, and what would change — query shape, entity requirements, mass scaling of existing RON values, networking/prediction behavior, and schedule ordering?
