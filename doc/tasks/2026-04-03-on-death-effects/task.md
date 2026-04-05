# On-Death Effects for World Objects

Add an `OnDeathEffects` component that can be attached to world objects (via `.object.ron` definitions) to trigger a list of effects when the object dies. Primary use cases: replacing a tree with a stump on death, respawning the tree after a delay, dropping resource items (items deferred to a future plan). Effects must replicate correctly through lightyear networking — the server is authoritative over death detection, effect execution, and entity spawning.

## Requirements

- **Persistence**: Introduce a `ReflectPersist` type data marker (new to the codebase) so that components marked with `#[reflect(Persist)]` are saved/restored during chunk eviction and on-death-effect transitions. External types use `app.register_type_data::<T, ReflectPersist>()`. Ensures persistent state survives world object transformations (e.g. tree → stump).

- **Spawn-time vs. reload safety**: Must distinguish first-spawn operations from reload operations. `PlacementOffset` currently re-applies on every reload, causing cumulative position drift (trees offset higher each load). The solution must prevent this class of bug.

- **Map instance / room correctness**: Newly spawned entities (e.g. stump replacing tree) must be placed in the correct lightyear room and associated with the correct `MapInstanceId`.

- **Deterministic sync**: Effects that involve randomness (e.g. random loot drops) must be deterministically synchronized between server and clients, or handled purely server-authoritative with replication.

- **Replication safety**: Must account for lightyear entity/child-entity replication ordering and avoid race conditions when despawning and spawning replacement entities. Consider whether server/client world object component sets can be symmetric or what compensation is needed.
