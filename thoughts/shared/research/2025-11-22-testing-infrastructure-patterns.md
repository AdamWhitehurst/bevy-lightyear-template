---
date: 2025-11-22T13:48:53-08:00
researcher: aw
git_commit: 1e807041c1ef77b7eac4f51d8c31909437791682
branch: master
repository: bevy-lightyear-template
topic: "Testing Infrastructure Setup for Server/Client/Web Connections"
tags: [research, testing, lightyear, bevy, integration-tests, networking, test-strategy, rust-testing, ci-cd, architecture-review]
status: complete
last_updated: 2025-11-22T13:55:11-08:00
last_updated_by: aw
last_updated_note: "Added specialized agent analysis: test strategy, Rust recommendations, implementation plan, architecture review"
---

# Research: Testing Infrastructure Setup for Server/Client/Web Connections

**Date**: 2025-11-22T13:48:53-08:00
**Researcher**: aw
**Git Commit**: 1e807041c1ef77b7eac4f51d8c31909437791682
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to set up testing code infrastructure to:
- Ensure server, client, web connection works
- Support testing future code, plugins, architecture
- Not rely on logs whenever possible
- Not rely on manual testing
- Enable agents to run tests programmatically so human user testing not required for most testing

## Summary

The current project has **no test infrastructure** implemented. However, the git submodules for Bevy and Lightyear contain extensive testing patterns specifically designed for the exact use cases requested:

1. **Lightyear** provides `ClientServerStepper` - a comprehensive test infrastructure for networked client-server testing with programmatic time control
2. **Bevy** provides `App`-based testing patterns for ECS systems, plugins, and integration testing without rendering
3. Both dependencies include test utilities designed for automated, programmatic testing without manual intervention or log reliance

## Current Project State

### Test Infrastructure Status
**Location**: All project crates (server, client, web, protocol)
**Status**: Zero test infrastructure exists

- No test modules (`#[cfg(test)]`)
- No integration tests (`tests/` directories)
- No dev-dependencies in any `Cargo.toml` files
- No CI/CD test configurations
- No test utilities or helpers

### Cargo Configuration Analysis

**Server** (`crates/server/Cargo.toml`):
- Dev Dependencies: None
- Dependencies: bevy, lightyear (server, netcode, udp, webtransport, websocket features), protocol, anyhow

**Client** (`crates/client/Cargo.toml`):
- Dev Dependencies: None
- Dependencies: bevy, lightyear (client, netcode, udp features), protocol, anyhow

**Web** (`crates/web/Cargo.toml`):
- Dev Dependencies: None
- Dependencies: bevy (bevy_winit, webgl2), lightyear (client, netcode, webtransport, websocket), protocol
- WASM Dependencies: wasm-bindgen, console_error_panic_hook, getrandom

**Protocol** (`crates/protocol/Cargo.toml`):
- Dev Dependencies: None
- Dependencies: bevy, lightyear, serde (derive feature)

## Detailed Findings

### Lightyear Testing Patterns

Lightyear's testing infrastructure is specifically designed for networked game testing and addresses all requirements.

#### Core Test Infrastructure: ClientServerStepper

**Location**: `git/lightyear/lightyear_tests/src/stepper.rs:36-567`

The `ClientServerStepper` is the primary testing utility that enables:
- Multiple independent client `App` instances
- Separate server `App` instance
- Crossbeam channel-based communication (no network stack required)
- Programmatic time control (frame stepping, tick stepping)
- Automatic connection/synchronization waiting
- Support for different client types (Host, Raw, Netcode, Steam)

**Example Setup**:
```rust
use lightyear::prelude::*;

#[test]
fn test_connection() {
    let mut stepper = ClientServerStepper::from_config(StepperConfig::single());

    // Automatic connection waiting
    stepper.wait_for_connection();
    stepper.wait_for_sync();

    // Verify network components present
    assert!(stepper.client(0).contains::<Connected>());
    assert!(stepper.server().contains::<Started>());
}
```

**Key Methods**:
- `from_config(config)` - Create stepper with predefined or custom config
- `wait_for_connection()` - Advance until all clients connected
- `wait_for_sync()` - Advance until clients synced
- `frame_step(n)` - Advance by n frames
- `tick_step(n)` - Advance by n ticks
- `client(id)` - Access client entity
- `server()` - Access server entity
- `client_of(id)` - Access server's client representation

#### Predefined Test Configurations

**Location**: `git/lightyear/lightyear_tests/src/stepper.rs:96-137`

```rust
// Single netcode client
StepperConfig::single()

// Host server + one remote client
StepperConfig::host_server()

// Multiple netcode clients
StepperConfig::with_netcode_clients(n)

// Custom configuration
StepperConfig {
    frame_duration: TICK_DURATION,
    tick_duration: TICK_DURATION,
    clients: vec![ClientType::Netcode, ClientType::Host],
    server: ServerType::Netcode,
    init: true,
    avian_mode: AvianReplicationMode::default(),
}
```

#### Entity Replication Testing

**Location**: `git/lightyear/lightyear_tests/src/client_server/replication.rs:17-106`

Tests entity spawning, despawning, and replication across network:

```rust
#[test]
fn test_spawn() {
    let mut stepper = ClientServerStepper::from_config(StepperConfig::single());

    let client_entity = stepper
        .client_app()
        .world_mut()
        .spawn((Replicate::to_server(),))
        .id();

    stepper.frame_step(1);

    // Verify entity replicated to server
    stepper
        .client_of(0)
        .get::<MessageManager>()
        .unwrap()
        .entity_mapper
        .get_local(client_entity)
        .expect("entity is not present in entity map");
}
```

**Replication Patterns**:
- `Replicate::to_server()` - Client-to-server replication
- `Replicate::to_clients(NetworkTarget::All)` - Server-to-all-clients
- `MessageManager.entity_mapper` - Maps entities across network boundaries

#### Message Testing

**Location**: `git/lightyear/lightyear_tests/src/client_server/messages.rs:32-80`

Bidirectional message testing with buffer pattern:

```rust
#[derive(Resource)]
struct Buffer<M>(Vec<(Entity, M)>);

fn count_messages_observer<M: Message + Debug>(
    mut receiver: Query<(Entity, &mut MessageReceiver<M>)>,
    mut buffer: ResMut<Buffer<M>>,
) {
    receiver.iter_mut().for_each(|(entity, mut receiver)| {
        receiver.receive().for_each(|m| buffer.0.push((entity, m)));
    })
}

#[test]
fn test_send_messages() {
    let mut stepper = ClientServerStepper::from_config(StepperConfig::single());
    stepper.server_app.init_resource::<Buffer<StringMessage>>();
    stepper
        .server_app
        .add_systems(Update, count_messages_observer::<StringMessage>);

    // Send message from client to server
    stepper
        .client_mut(0)
        .get_mut::<MessageSender<StringMessage>>()
        .unwrap()
        .send::<Channel1>(send_message.clone());

    stepper.frame_step(1);

    // Verify message received
    let received_messages = stepper
        .server_app
        .world()
        .resource::<Buffer<StringMessage>>();
    assert_eq!(
        &received_messages.0,
        &vec![(stepper.client_of_entities[0], send_message)]
    );
}
```

**Pattern**: Buffer resource for collecting networked messages during test execution

#### Input Testing

**Location**: `git/lightyear/lightyear_tests/src/client_server/input/native.rs:20-90`

Tests networked input handling and synchronization:

```rust
#[test]
fn test_remote_client_replicated_input() {
    let mut stepper = ClientServerStepper::from_config(StepperConfig::single());

    // Setup replicated entity
    let server_entity = stepper
        .server_app
        .world_mut()
        .spawn(Replicate::to_clients(NetworkTarget::All))
        .id();

    stepper.frame_step(2);

    let client_entity = stepper
        .client(0)
        .get::<MessageManager>()
        .unwrap()
        .entity_mapper
        .get_local(server_entity)
        .expect("entity was not replicated to client");

    // Set input on client
    stepper
        .client_app()
        .world_mut()
        .entity_mut(client_entity)
        .insert(InputMarker::<MyInput>::default());
    stepper
        .client_app()
        .world_mut()
        .get_mut::<ActionState<MyInput>>(client_entity)
        .unwrap()
        .0 = MyInput(1);

    stepper.frame_step(1);
    let client_tick = stepper.client_tick(0);

    // Verify input received on server
    assert_eq!(
        stepper
            .server_app
            .world()
            .get::<NativeBuffer<MyInput>>(server_entity)
            .unwrap()
            .get(client_tick)
            .unwrap(),
        &ActionState(MyInput(1))
    );
}
```

**Pattern**: Tick-synchronized input testing with buffer verification

#### Protocol Definition for Tests

**Location**: `git/lightyear/lightyear_tests/src/protocol.rs:1-220`

Defines networked messages, components, and channels:

```rust
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

// Messages
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Reflect)]
pub struct StringMessage(pub String);

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, MapEntities, Reflect)]
pub struct EntityMessage(#[entities] pub Entity);

// Channels
#[derive(Reflect)]
pub struct Channel1;

// Components with interpolation
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
pub struct CompFull(pub f32);

impl Ease for CompFull {
    fn interpolating_curve_unbounded(start: Self, end: Self) -> impl Curve<Self> {
        FunctionCurve::new(Interval::UNIT, move |t| {
            CompFull(f32::lerp(start.0, end.0, t))
        })
    }
}

// Protocol registration
impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.register_message::<StringMessage>()
            .add_direction(NetworkDirection::Bidirectional);

        app.add_channel::<Channel1>(ChannelSettings {
            mode: ChannelMode::UnorderedUnreliable,
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);

        app.register_component::<CompFull>()
            .add_prediction()
            .add_linear_interpolation();
    }
}
```

#### Test Utilities Feature Flag

**Location**: `git/lightyear/lightyear_tests/Cargo.toml:11-30`

```toml
[features]
default = ["std", "test_utils"]
test_utils = [
    "lightyear_core/test_utils",
    "lightyear_transport/test_utils",
    "lightyear_replication/test_utils",
    "lightyear_crossbeam/test_utils",
    "lightyear_aeronet/test_utils",
]
```

### Bevy Testing Patterns

Bevy provides comprehensive patterns for testing ECS systems, plugins, and apps without rendering.

#### Basic System Testing with App

**Location**: `git/bevy/tests/how_to_test_systems.rs:50-79`

```rust
#[test]
fn did_hurt_enemy() {
    let mut app = App::new();

    app.insert_resource(Score(0));
    app.add_message::<EnemyDied>();
    app.add_systems(Update, (hurt_enemies, despawn_dead_enemies).chain());

    let enemy_id = app
        .world_mut()
        .spawn(Enemy {
            hit_points: 5,
            score_value: 3,
        })
        .id();

    // Run systems programmatically
    app.update();

    // Verify results without logs
    assert!(app.world().get::<Enemy>(enemy_id).is_some());
    assert_eq!(app.world().get::<Enemy>(enemy_id).unwrap().hit_points, 4);
}
```

**Pattern**: Create `App`, add systems, run `update()`, verify with assertions

#### Headless App Testing with Input Simulation

**Location**: `git/bevy/tests/how_to_test_apps.rs:52-86`

```rust
fn create_test_app() -> App {
    let mut app = App::new();

    // MinimalPlugins = headless (no rendering)
    app.add_plugins(MinimalPlugins);

    // Inject input for testing
    app.insert_resource(ButtonInput::<KeyCode>::default());

    // Fake window for systems that require it
    app.world_mut().spawn(Window::default());

    app
}

#[test]
fn test_spell_casting() {
    let mut app = create_test_app();
    app.add_plugins(game_plugin);

    // Simulate input
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Space);
    app.update();

    // Verify behavior
    let actual = app
        .world_mut()
        .query::<&Player>()
        .single(app.world())
        .unwrap();
    assert_eq!(
        Player::default().mana - 1,
        actual.mana,
        "A single mana point should have been used."
    );

    // Clear input state
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    app.update();

    // Verify no further changes
    let after = app
        .world_mut()
        .query::<&Player>()
        .single(app.world())
        .unwrap();
    assert_eq!(Player::default().mana - 1, after.mana);
}
```

**Pattern**: Use `MinimalPlugins` for headless testing, simulate input with resource manipulation

#### Plugin Testing

**Location**: `git/bevy/crates/bevy_app/src/propagate.rs:302-327`

```rust
#[test]
fn test_simple_propagate() {
    let mut app = App::new();
    app.add_schedule(Schedule::new(Update));
    app.add_plugins(HierarchyPropagatePlugin::<TestValue>::new(Update));

    let propagator = app.world_mut().spawn(Propagate(TestValue(1))).id();
    let intermediate = app
        .world_mut()
        .spawn_empty()
        .insert(ChildOf(propagator))
        .id();
    let propagatee = app
        .world_mut()
        .spawn_empty()
        .insert(ChildOf(intermediate))
        .id();

    app.update();

    assert!(app
        .world_mut()
        .query::<&TestValue>()
        .get(app.world(), propagatee)
        .is_ok());
}
```

**Pattern**: Test plugins in isolation with custom schedules

#### Event/Message Testing

**Location**: `git/bevy/tests/how_to_test_systems.rs:150-174`

```rust
#[test]
fn update_score_on_event() {
    let mut app = App::new();
    app.insert_resource(Score(0));
    app.add_message::<EnemyDied>();
    app.add_systems(Update, update_score);

    // Write event programmatically
    app.world_mut()
        .resource_mut::<Messages<EnemyDied>>()
        .write(EnemyDied(3));

    app.update();

    // Verify event handling
    assert_eq!(app.world().resource::<Score>().0, 3);
}
```

**Pattern**: Programmatically write events, verify effects with assertions

#### Direct World Testing (Low-Level)

**Location**: `git/bevy/crates/bevy_ecs/src/query/mod.rs:139-153`

```rust
#[test]
fn query() {
    let mut world = World::new();
    world.spawn((A(1), B(1)));
    world.spawn(A(2));

    let values = world.query::<&A>().iter(&world).collect::<HashSet<&A>>();
    assert!(values.contains(&A(1)));
    assert!(values.contains(&A(2)));

    for (_a, mut b) in world.query::<(&A, &mut B)>().iter_mut(&mut world) {
        b.0 = 3;
    }

    let values = world.query::<&B>().iter(&world).collect::<Vec<&B>>();
    assert_eq!(values, vec![&B(3)]);
}
```

**Pattern**: Test ECS queries without full App overhead

## Code References

### Lightyear Testing Infrastructure
- `git/lightyear/lightyear_tests/src/stepper.rs:36-567` - ClientServerStepper core implementation
- `git/lightyear/lightyear_tests/src/stepper.rs:96-137` - StepperConfig presets
- `git/lightyear/lightyear_tests/src/client_server/base.rs:9-105` - Connection setup tests
- `git/lightyear/lightyear_tests/src/client_server/messages.rs:32-80` - Message testing patterns
- `git/lightyear/lightyear_tests/src/client_server/replication.rs:17-106` - Entity replication tests
- `git/lightyear/lightyear_tests/src/client_server/input/native.rs:20-90` - Input synchronization tests
- `git/lightyear/lightyear_tests/src/protocol.rs:1-220` - Test protocol definitions
- `git/lightyear/lightyear_tests/Cargo.toml:11-30` - Test utilities feature flag

### Bevy Testing Infrastructure
- `git/bevy/tests/how_to_test_systems.rs:50-79` - Basic system testing
- `git/bevy/tests/how_to_test_systems.rs:81-108` - Multi-system integration
- `git/bevy/tests/how_to_test_systems.rs:150-174` - Event testing
- `git/bevy/tests/how_to_test_apps.rs:52-86` - Headless app testing with input simulation
- `git/bevy/crates/bevy_app/src/propagate.rs:302-327` - Plugin testing
- `git/bevy/crates/bevy_ecs/src/query/mod.rs:139-153` - Direct world testing
- `git/bevy/crates/bevy_app/src/task_pool_plugin.rs:264-295` - Async task pool testing

### Current Project
- `crates/server/Cargo.toml:1-11` - Server crate (no test config)
- `crates/client/Cargo.toml:1-11` - Client crate (no test config)
- `crates/web/Cargo.toml:1-31` - Web crate (no test config)
- `crates/protocol/Cargo.toml:1-10` - Protocol crate (no test config)
- `Cargo.toml:1-20` - Workspace root (no test config)

## Architecture Documentation

### Testing Architecture Patterns Found

#### 1. Multi-App Pattern (Lightyear)
- Separate `App` instances for each network endpoint
- Crossbeam channels for inter-app communication
- Independent time control via frame/tick stepping
- Eliminates need for actual network stack in tests

#### 2. Headless Testing Pattern (Bevy)
- `MinimalPlugins` instead of `DefaultPlugins`
- No window system, rendering, or audio
- Full ECS functionality for system testing
- Resource injection for input/state simulation

#### 3. Buffer Pattern (Lightyear Messages)
- Resource-based message collection
- Observer systems for gathering test data
- Non-intrusive test instrumentation

#### 4. Entity Mapper Pattern (Lightyear)
- Tracks entity correspondence across network
- Accessible via `MessageManager.entity_mapper`
- Essential for verifying replication

#### 5. Programmatic Time Control (Lightyear)
- `frame_step(n)` - Advance by frame durations
- `tick_step(n)` - Advance by tick durations
- `wait_for_connection()` - Auto-advance until connected
- `wait_for_sync()` - Auto-advance until synced

### Test Execution Flow

```
1. Setup:
   - Create test App(s)
   - Configure plugins/resources/systems
   - Spawn test entities

2. Action:
   - Inject state (messages, input, components)
   - Call app.update() or stepper methods
   - Programmatically advance time

3. Verification:
   - Query world state
   - Assert component values
   - Check resource state
   - Verify entity existence/relationships

4. No manual intervention or log parsing required
```

### Common Testing Utilities

**Lightyear**:
- `ClientServerStepper` - Main test harness
- `StepperConfig` - Configuration builder
- `MessageManager.entity_mapper` - Entity correspondence
- `Buffer<T>` resource - Message collection
- `test_log::test` - Test logging macro

**Bevy**:
- `App::new()` - Create test app
- `MinimalPlugins` - Headless plugin set
- `app.update()` - Run one frame
- `world.query::<T>()` - Query entities
- `world.spawn(bundle)` - Spawn test entities
- `ButtonInput::<KeyCode>` - Input simulation
- `Messages<T>` - Event system

## Related Research

None yet - this is the first research document on testing infrastructure.

## Open Questions

1. **WASM Testing**: How to test the web crate's WASM-specific code? Are there patterns in Bevy/Lightyear for WASM target testing?

2. **CI/CD Integration**: What test runner configurations work best for cargo-make integration? Should tests be in separate tasks?

3. **Test Organization**: Should tests live in crate-specific `tests/` directories, or in a workspace-level `tests/` directory?

4. **Performance Testing**: Are there patterns for benchmarking networked game performance (tick rates, latency simulation)?

5. **Avian Integration**: How to test physics replication with avian3d? Does Lightyear have specific patterns for physics testing?

6. **Multi-Client Scenarios**: What's the practical limit for number of simulated clients in `ClientServerStepper`?

---

## Follow-up Research: Specialized Agent Analysis

**Date**: 2025-11-22T13:55:11-08:00

After documenting existing testing patterns, specialized agents provided comprehensive analysis for test strategy, Rust-specific recommendations, implementation planning, and architecture review.

### Test Strategy (qa-expert)

**Test Pyramid Structure**:
- Unit Tests: 60% coverage target
- Integration Tests: 30% coverage target
- End-to-End Tests: 10% coverage target

**Coverage Targets by Crate**:
- Protocol: 95% (foundational crate)
- Server: 85% (business logic)
- Client: 80% (UI complexity)
- Web: 70% (WASM limitations)

**Test Organization Recommendation**:
```
crates/
  protocol/tests/     - Serialization, registration tests
  server/tests/       - Multi-transport, client lifecycle
  client/tests/       - Connection, reconnection
  web/tests/          - WASM integration (conditional)
tests/                - Workspace-level integration tests
  common/             - Shared test utilities
  connection_flows.rs - E2E connection testing
  multi_client.rs     - Multi-client scenarios
benches/              - Performance benchmarks
```

**WASM Testing Strategy**:
- Use wasm-bindgen-test for browser-based execution
- Conditional compilation for WASM vs native tests
- Smoke tests only (initialization, compilation verification)
- Full E2E in native client tests
- CI integration with wasm-pack

**Key Makefile.toml Tasks**:
- `test` - Run all tests
- `test-unit` - Unit tests only
- `test-integration` - Integration tests only
- `test-wasm` - WASM-specific tests
- `test-ci` - Full CI test suite
- `test-coverage` - Generate coverage reports

### Rust-Specific Recommendations (rust-engineer)

**Required Dev-Dependencies**:

Workspace-level:
```toml
[workspace.dependencies]
mock_instant = "0.5"
test-log = { version = "0.2", features = ["trace"] }
approx = "0.5"
```

Protocol crate:
```toml
[dev-dependencies]
test-log = { workspace = true }
approx = { workspace = true }
```

Server/Client crates:
```toml
[dev-dependencies]
lightyear = { workspace = true, features = ["test_utils"] }
test-log = { workspace = true }
mock_instant = { workspace = true }
```

Web crate:
```toml
[dev-dependencies]
lightyear = { workspace = true, features = ["test_utils"] }
wasm-bindgen-test = "0.3"
```

**Testing Pattern Locations**:
- `#[cfg(test)]` modules - Inline unit tests in src/ files
- `tests/` directories - Integration tests per crate
- `benches/` - Performance benchmarks with criterion

**Async Testing**: Use `ClientServerStepper` programmatic time control (not async runtime)

**Feature Flags**:
```toml
[features]
test_helpers = []  # Enable test-only components

[target.'cfg(test)'.dependencies.lightyear]
features = ["test_utils"]  # Only in test builds
```

**Performance Testing Setup**:
- Create separate `benches/` crate
- Use criterion with html_reports feature
- Benchmark replication, message throughput
- Add `cargo bench` task to Makefile.toml

### Implementation Plan (test-automator)

**Directory Structure**:
```
crates/
  test-utils/           # NEW: Shared test utilities crate
    src/
      lib.rs            # Public API
      builders.rs       # Test data builders
      fixtures.rs       # Common test fixtures
      helpers.rs        # Helper functions
```

**Test Utilities Crate** (`crates/test-utils/Cargo.toml`):
```toml
[package]
name = "test-utils"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true }
lightyear = { workspace = true, features = ["client", "server", "crossbeam", "netcode", "test_utils"] }
protocol = { workspace = true }

[dev-dependencies]
approx = "0.5"
```

**Key Test Utilities**:
- `TestStepperBuilder` - Fluent API for stepper configuration
- `default_single_client()` - Pre-configured single client stepper
- `default_multi_client(n)` - Pre-configured multi-client stepper
- `assert_connected()` - Assert client connection
- `assert_synced()` - Assert client synchronization
- `assert_entity_replicated()` - Assert entity replication
- `collect_messages<M>()` - Message collection helper

**CI/CD Pipeline** (`.github/workflows/ci.yml`):
```yaml
jobs:
  test:
    - Run unit tests: cargo test --workspace --lib
    - Run integration tests: cargo test --workspace --tests
  test-wasm:
    - Install wasm-pack
    - Run WASM tests: wasm-pack test --headless --firefox
  coverage:
    - Generate with cargo-llvm-cov
    - Upload to codecov
  lint:
    - Check formatting: cargo fmt --check
    - Run clippy: cargo clippy -- -D warnings
```

**Test Parallelization**:
- Cargo default: One thread per CPU core
- Each crate runs independently
- Manual override: `cargo test --jobs 4 -- --test-threads 4`
- Isolation via separate App instances per test

**Implementation Task Order**:
1. Create test-utils crate
2. Update workspace Cargo.toml
3. Implement test-utils modules
4. Add dev-dependencies to all crates
5. Create integration test directories
6. Write initial integration tests
7. Add Makefile.toml test tasks
8. Create CI/CD workflow
9. Write per-crate unit tests
10. Setup WASM tests
11. Configure coverage
12. Document testing patterns

### Architecture Review (architect-reviewer)

**Architectural Fit Assessment**: ✅ APPROVED with conditions

**Strong Alignment**:
- Protocol crate maps directly to Lightyear test protocol pattern
- Server multi-transport design supports ClientServerStepper client types
- Client connection observers testable programmatically
- In-memory channels eliminate network flakiness

**Scalability Analysis**:
- Positive: Crossbeam channels (no network overhead)
- Positive: Headless testing (no rendering bottleneck)
- Positive: Deterministic time control
- Concern: Multiple stepper instances may hit memory limits
- **Score**: 9/10 (excellent)

**Test Boundaries**:
- Protocol: Unit tests (no networking)
- Server/Client: Integration tests with ClientServerStepper
- Web: Native-compiled network tests + WASM smoke tests
- Workspace: Full system end-to-end tests

**Dependency Management Recommendation**:
- **Option A** (Recommended): Protocol crate dev-dependencies
  - Centralizes test utilities in shared crate
  - Access via `protocol::test_utils` module
  - Follows Lightyear's architecture
- Option B: Separate test-utils crate (only if complexity warrants)

**Isolation Score**: 9/10
- Per-test App instances (no shared state)
- Crossbeam channels scoped to stepper lifetime
- Parallel execution safe
- No port binding conflicts (tests use channels)

**Performance Projection**:
- 100 tests: ~5 seconds
- 1000 tests: ~30 seconds
- Sub-millisecond per connection test
- Microseconds per frame step

**Architectural Risks & Mitigations**:

1. **WASM Test Coverage Gap** (High severity)
   - Mitigation: Native builds with WASM feature flags, wasm-pack test research, manual WASM testing initially

2. **Multi-Transport Test Complexity** (Medium severity)
   - Mitigation: Per-transport stepper configs, parameterized tests, shared behavior tests

3. **Lightyear Version Dependency** (Medium severity)
   - Mitigation: Pin version (0.25.5), git submodule validation, upgrade checklist

4. **Missing Future Feature Tests** (Low severity)
   - Mitigation: Phase 1 (connections), Phase 2 (replication), Phase 3 (physics/prediction)

5. **Test-Production Drift** (Medium severity)
   - Mitigation: Same ProtocolPlugin, observer patterns match, CI on every commit

**Implementation Phases**:

Phase 1 (Immediate):
- Add lightyear test_utils to protocol dev-deps
- Basic connection tests (server, client)
- Protocol registration tests
- Makefile.toml CI task

Phase 2 (Short-term):
- Multi-transport testing
- Observer lifecycle tests
- Bidirectional message tests
- Test utilities module
- WASM testing research

Phase 3 (Long-term):
- Entity replication tests
- Physics replication (Avian)
- Performance benchmarking
- Workspace-level E2E tests

**Confidence Level**: 8/10
**Blocker Status**: None

**Critical Path Dependencies**:
1. Solve WASM testing gap (deferrable with mitigation)
2. Configure CI integration (required for team development)
3. Document multi-transport testing strategy

### Answers to Open Questions

Based on specialized agent analysis:

**1. WASM Testing** - ✅ ADDRESSED
- Use wasm-bindgen-test with headless browser execution
- Conditional compilation for WASM vs native
- Native builds test network logic, WASM tests verify compilation
- wasm-pack integration in CI

**2. CI/CD Integration** - ✅ ADDRESSED
- Separate Makefile.toml tasks per test type
- GitHub Actions workflow with test, test-wasm, coverage, lint jobs
- Explicit cargo-make task dependencies

**3. Test Organization** - ✅ ADDRESSED
- Crate-specific `tests/` for integration tests
- Workspace-level `tests/` for full-stack E2E
- `crates/test-utils/` for shared utilities

**4. Performance Testing** - ✅ ADDRESSED
- Criterion benchmarks in `benches/` crate
- Benchmark replication and message throughput
- Deferred until functional correctness complete

**5. Avian Integration** - 🔄 DEFERRED
- Reference Lightyear avian example tests when needed
- Phase 3 implementation (after basic replication working)

**6. Multi-Client Scenarios** - 🔄 OPEN
- Practical limit TBD (memory-bound, not CPU-bound)
- Start with 2-5 clients, scale as needed
- Parameterized tests for multi-client scenarios
