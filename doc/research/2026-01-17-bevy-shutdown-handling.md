---
date: 2026-01-17
researcher: rust-engineer
git_commit: 343ab540f013d222150301c9670701f5d2dcb4dd
branch: master
repository: bevy-lightyear-template
topic: "Bevy graceful shutdown handling and cleanup patterns"
tags: [research, bevy, shutdown, cleanup, appexit, best-practices]
status: complete
last_updated: 2026-01-17
last_updated_by: rust-engineer
---

# Research: Bevy Graceful Shutdown Handling

**Date**: 2026-01-17
**Bevy Version**: 0.17.2 (project), 0.18.0 (latest)
**Source**: Bevy git repository at `/home/aw/Dev/bevy-lightyear-template/git/bevy`

## Research Question

How to properly handle graceful shutdown in Bevy applications, specifically:
1. How to detect AppExit events
2. Best practices for cleanup systems that run on shutdown
3. Whether to use OnExit schedules or AppExit events
4. Caveats with file I/O during shutdown

## Summary

Bevy uses a **message-based** (not event-based) system for application exit via `AppExit` messages. Cleanup systems should use `MessageReader<AppExit>` or `MessageWriter<AppExit>` in normal schedules (typically `Update` or `Last`), **not** `OnExit` state schedules. File I/O during shutdown works reliably in the main app loop before exit, but can fail in render world systems where resources may be dropped.

## AppExit Message Type

### Definition

Location: `/home/aw/Dev/bevy-lightyear-template/git/bevy/crates/bevy_app/src/app.rs:1475-1506`

```rust
/// Message used to cause a Bevy app to exit.
///
/// An app will exit after the [runner](App::set_runner) will end and return control to the caller.
///
/// This message can be used to detect when an exit is requested. Make sure that systems listening
/// for this message run before the current update ends.
#[derive(Message, Debug, Clone, Default, PartialEq, Eq)]
pub enum AppExit {
    /// [`App`] exited without any problems.
    #[default]
    Success,
    /// The [`App`] experienced an unhandleable error.
    /// Holds the exit code we expect our app to return.
    Error(NonZero<u8>),
}

impl AppExit {
    /// Creates a [`AppExit::Error`] with an error code of 1.
    #[must_use]
    pub const fn error() -> Self {
        Self::Error(NonZero::<u8>::MIN)
    }

    /// Returns `true` if `self` is a [`AppExit::Success`].
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, AppExit::Success)
    }

    /// Returns `true` if `self` is a [`AppExit::Error`].
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self, AppExit::Error(_))
    }

    /// Creates a [`AppExit`] from a code.
    /// When `code` is 0 a [`AppExit::Success`] is constructed otherwise a
    /// [`AppExit::Error`] with the provided code.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => AppExit::Success,
            _ => AppExit::Error(NonZero::new(code).unwrap()),
        }
    }
}
```

**Key Characteristics**:
- Uses `#[derive(Message)]` not `Event`
- Accessed via `MessageWriter<AppExit>` (to send) and `MessageReader<AppExit>` (to check)
- Two variants: `Success` (code 0) and `Error(NonZero<u8>)` (codes 1-255)
- Mapped to standard process exit codes

## How to Trigger AppExit

### Writing AppExit Messages

Use `MessageWriter<AppExit>` system parameter:

```rust
fn trigger_exit(mut app_exit: MessageWriter<AppExit>) {
    app_exit.write(AppExit::Success);
}
```

From examples in codebase:

**`git/bevy/examples/app/headless_renderer.rs:478-540`**:
```rust
fn update(
    images_to_save: Query<&ImageToSave>,
    receiver: Res<MainWorldReceiver>,
    mut images: ResMut<Assets<Image>>,
    mut scene_controller: ResMut<SceneController>,
    mut app_exit_writer: MessageWriter<AppExit>,
    mut file_number: Local<u32>,
) {
    // ... save image to file ...
    if scene_controller.single_image {
        app_exit_writer.write(AppExit::Success);
    }
}
```

**`git/bevy/examples/games/game_menu.rs:686-700`**:
```rust
fn menu_action(
    interaction_query: Query<
        (&Interaction, &MenuButtonAction),
        (Changed<Interaction>, With<Button>),
    >,
    mut app_exit_writer: MessageWriter<AppExit>,
    mut menu_state: ResMut<NextState<MenuState>>,
    mut game_state: ResMut<NextState<GameState>>,
) {
    for (interaction, menu_button_action) in &interaction_query {
        if *interaction == Interaction::Pressed {
            match menu_button_action {
                MenuButtonAction::Quit => {
                    app_exit_writer.write(AppExit::Success);
                }
                // ... other actions ...
            }
        }
    }
}
```

### Ctrl+C Handling

Built-in via `TerminalCtrlCHandlerPlugin`:

**`git/bevy/crates/bevy_app/src/terminal_ctrl_c_handler.rs:41-74`**:
```rust
#[derive(Default)]
pub struct TerminalCtrlCHandlerPlugin;

impl TerminalCtrlCHandlerPlugin {
    /// Sends the [`AppExit`] event to all apps using this plugin to make them gracefully exit.
    pub fn gracefully_exit() {
        SHOULD_EXIT.store(true, Ordering::Relaxed);
    }

    /// Sends a [`AppExit`] event when the user presses `Ctrl+C` on the terminal.
    pub fn exit_on_flag(mut app_exit_writer: MessageWriter<AppExit>) {
        if SHOULD_EXIT.load(Ordering::Relaxed) {
            app_exit_writer.write(AppExit::from_code(130));
        }
    }
}

impl Plugin for TerminalCtrlCHandlerPlugin {
    fn build(&self, app: &mut App) {
        let result = ctrlc::try_set_handler(move || {
            Self::gracefully_exit();
        });
        // ... error handling ...
        app.add_systems(Update, TerminalCtrlCHandlerPlugin::exit_on_flag);
    }
}
```

**Pattern**:
- Sets atomic flag in signal handler
- System in `Update` checks flag each frame and writes `AppExit`
- Exit code 130 (standard for SIGINT)

## How to Detect AppExit

### Reading AppExit Messages

Use `MessageReader<AppExit>` system parameter:

```rust
fn on_shutdown(
    mut exit_reader: MessageReader<AppExit>,
    // ... other parameters ...
) {
    if !exit_reader.is_empty() {
        // AppExit message(s) present
        for exit in exit_reader.read() {
            if exit.is_success() {
                // Clean exit
            } else {
                // Error exit
            }
        }
    }
}
```

**Important**: `MessageReader` only shows messages written in current frame. Must run in same update cycle.

### Example: Window Cleanup on Exit

**`git/bevy/crates/bevy_winit/src/system.rs:241-283`**:
```rust
pub(crate) fn despawn_windows(
    closing: Query<Entity, With<ClosingWindow>>,
    mut closed: RemovedComponents<Window>,
    window_entities: Query<Entity, With<Window>>,
    mut closing_event_writer: MessageWriter<WindowClosing>,
    mut closed_event_writer: MessageWriter<WindowClosed>,
    mut windows_to_drop: Local<Vec<WindowWrapper<winit::window::Window>>>,
    mut exit_event_reader: MessageReader<AppExit>,
    _non_send_marker: NonSendMarker,
) {
    // Drop all the windows that are waiting to be closed
    windows_to_drop.clear();
    for window in closing.iter() {
        closing_event_writer.write(WindowClosing { window });
    }
    // ... handle closed windows ...

    // On macOS, when exiting, we need to tell the rendering thread the windows are about to
    // close to ensure that they are dropped on the main thread. Otherwise, the app will hang.
    if !exit_event_reader.is_empty() {
        exit_event_reader.clear();
        for window in window_entities.iter() {
            closing_event_writer.write(WindowClosing { window });
        }
    }
}
```

**Pattern**: Check `!exit_event_reader.is_empty()` then perform cleanup.

### Example: Automatic Exit on Window Close

**`git/bevy/crates/bevy_window/src/system.rs:13-26`**:
```rust
pub fn exit_on_all_closed(mut app_exit_writer: MessageWriter<AppExit>, windows: Query<&Window>) {
    if windows.is_empty() {
        info!("No windows are open; exiting.");
        app_exit_writer.write(AppExit::Success);
    }
}

pub fn exit_on_primary_closed(
    mut app_exit_writer: MessageWriter<AppExit>,
    windows: Query<&Window>,
) {
    if !windows.iter().any(|window| window.primary) {
        info!("Primary window has been closed; exiting.");
        app_exit_writer.write(AppExit::Success);
    }
}
```

## Schedules and Timing

### Main Schedule Order

**`git/bevy/crates/bevy_app/src/main_schedule.rs:12-38`**:

Order within each update tick:
1. `First`
2. `PreUpdate`
3. `StateTransition` (if bevy_state enabled)
4. `RunFixedMainLoop` (contains FixedMain substeps)
5. `Update`
6. `SpawnScene`
7. `PostUpdate`
8. `Last`

**Startup phase** (first frame only):
1. `StateTransition`
2. `PreStartup`
3. `Startup`
4. `PostStartup`

### Recommended Schedule for Cleanup

**Use `Last` schedule** for shutdown cleanup:

```rust
app.add_systems(Last, save_on_shutdown);
```

**Rationale**:
- Runs after all game logic (`Update`, `PostUpdate`)
- Still within same frame as `AppExit` message
- State fully consistent
- All systems have completed their work

### Why NOT OnExit State Schedules

`OnExit` schedules are for **state transitions**, not application shutdown:

```rust
// This runs when transitioning OUT of GameState::InGame
app.add_systems(OnExit(GameState::InGame), cleanup_game);

// This does NOT run on AppExit
```

**Key differences**:
- `OnExit` = leaving a state (still running)
- `AppExit` = terminating entire application
- `OnExit` may not run if app exits while in that state
- States may not transition during shutdown

## File I/O During Shutdown

### Safe: Main World Systems

File I/O in main world schedules is **safe and reliable** during shutdown:

**Example from `git/bevy/examples/app/headless_renderer.rs:473-548`**:
```rust
fn update(
    images_to_save: Query<&ImageToSave>,
    receiver: Res<MainWorldReceiver>,
    mut images: ResMut<Assets<Image>>,
    mut scene_controller: ResMut<SceneController>,
    mut app_exit_writer: MessageWriter<AppExit>,
    mut file_number: Local<u32>,
) {
    // ... process image data ...

    // Heavy blocking I/O operation before exit
    if let Err(e) = img.save(image_path) {
        panic!("Failed to save image: {e}");
    };

    if scene_controller.single_image {
        app_exit_writer.write(AppExit::Success);  // Exit after save completes
    }
}
```

**Note in code**:
```rust
// Finally saving image to file, this heavy blocking operation is kept here
// for example simplicity, but in real app you should move it to a separate task
```

**Pattern**: File I/O completes **before** writing `AppExit`, guaranteeing completion.

### Unsafe: Render World Systems

File I/O in render world has **timing hazards**:

**From `git/bevy/examples/app/headless_renderer.rs:458`**:
```rust
// This could fail on app exit, if Main world clears resources (including receiver)
// while Render world still renders
let _ = sender.send(buffer_slice.get_mapped_range().to_vec());
```

**Problem**: Main world and render world run in parallel. On shutdown:
1. Main world may drop resources
2. Render world systems may still execute
3. Resource access fails

**Solution**: Perform critical I/O in main world, not render world.

## Best Practices

### 1. Cleanup System Pattern

```rust
fn save_on_shutdown(
    mut exit_reader: MessageReader<AppExit>,
    data: Res<GameData>,
) {
    if !exit_reader.is_empty() {
        // Only process first exit message
        exit_reader.clear();

        match save_to_file(&data) {
            Ok(_) => info!("Save complete"),
            Err(e) => error!("Save failed: {e}"),
        }
    }
}

app.add_systems(Last, save_on_shutdown);
```

**Rationale**:
- `Last` schedule ensures all game systems complete
- Check `is_empty()` before doing expensive work
- Call `clear()` to consume messages
- File I/O happens before exit completes
- Error handling doesn't prevent shutdown

### 2. Debounced Autosave + Shutdown Save

```rust
#[derive(Resource)]
struct SaveTimer {
    timer: Timer,
    dirty: bool,
}

fn mark_dirty_on_edit(
    mut save_timer: ResMut<SaveTimer>,
    edit_events: EventReader<VoxelEdit>,
) {
    if !edit_events.is_empty() {
        save_timer.dirty = true;
        save_timer.timer.reset();
    }
}

fn autosave_debounced(
    time: Res<Time>,
    mut save_timer: ResMut<SaveTimer>,
    data: Res<GameData>,
) {
    if save_timer.dirty {
        save_timer.timer.tick(time.delta());
        if save_timer.timer.finished() {
            save_to_file(&data);
            save_timer.dirty = false;
        }
    }
}

fn save_on_shutdown(
    mut exit_reader: MessageReader<AppExit>,
    save_timer: Res<SaveTimer>,
    data: Res<GameData>,
) {
    if !exit_reader.is_empty() {
        exit_reader.clear();
        if save_timer.dirty {
            save_to_file(&data);
        }
    }
}

app.add_systems(Update, (mark_dirty_on_edit, autosave_debounced));
app.add_systems(Last, save_on_shutdown);
```

**Pattern**: Regular autosave with final save on exit if dirty.

### 3. Atomic File Writes

```rust
use std::fs;
use std::io::Write;

fn save_to_file(data: &GameData) -> std::io::Result<()> {
    let path = "world_save/voxel_world.bin";
    let temp_path = format!("{path}.tmp");

    // Write to temporary file
    let bytes = bincode::serialize(data).unwrap();
    fs::write(&temp_path, bytes)?;

    // Atomic rename (prevents corruption)
    fs::rename(temp_path, path)?;

    Ok(())
}
```

**Rationale**: If interrupted, main file remains intact.

### 4. Resource Cleanup with Drop

For cleanup that must run even on panic:

```rust
#[derive(Resource)]
struct TempFileGuard {
    path: PathBuf,
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
```

**Pattern**: `Drop` runs on panic, `AppExit`, or normal drop.

## Common Patterns in Bevy Examples

### Pattern 1: Exit After Task Completion

```rust
fn check_work_complete(
    mut app_exit: MessageWriter<AppExit>,
    work: Res<WorkTracker>,
) {
    if work.is_complete() {
        app_exit.write(AppExit::Success);
    }
}
```

### Pattern 2: Exit on User Action

```rust
fn handle_exit_button(
    interaction_query: Query<&Interaction, (Changed<Interaction>, With<ExitButton>)>,
    mut app_exit: MessageWriter<AppExit>,
) {
    for interaction in &interaction_query {
        if *interaction == Interaction::Pressed {
            app_exit.write(AppExit::Success);
        }
    }
}
```

### Pattern 3: Conditional Exit on Window Event

```rust
fn exit_on_all_closed(
    mut app_exit: MessageWriter<AppExit>,
    windows: Query<&Window>
) {
    if windows.is_empty() {
        app_exit.write(AppExit::Success);
    }
}
```

## Caveats and Gotchas

### 1. MessageReader is Frame-Local

```rust
fn detect_exit(mut exit_reader: MessageReader<AppExit>) {
    // Only sees AppExit written THIS FRAME
    if !exit_reader.is_empty() {
        // Will only execute in frame where AppExit was written
    }
}
```

**Solution**: Add system to `Last` schedule to catch exits from earlier schedules.

### 2. Multiple AppExit Messages

```rust
fn first_system(mut app_exit: MessageWriter<AppExit>) {
    app_exit.write(AppExit::Success);
}

fn second_system(mut app_exit: MessageWriter<AppExit>) {
    app_exit.write(AppExit::Error(1.try_into().unwrap()));
}
```

**Result**: Both messages exist; last one wins for exit code.

**Best practice**: Write `AppExit` from single authoritative location.

### 3. Panics Don't Trigger AppExit

```rust
fn broken_system() {
    panic!("This does NOT write AppExit");
}
```

**Cleanup won't run**. Use `Drop` impl for critical cleanup.

### 4. Blocking I/O Blocks Frame

```rust
fn slow_save(mut exit_reader: MessageReader<AppExit>) {
    if !exit_reader.is_empty() {
        // This blocks entire app for 5 seconds
        thread::sleep(Duration::from_secs(5));
        save_large_file();
    }
}
```

**Acceptable on shutdown**, but not for autosave during gameplay.

### 5. Exit Condition Configuration

```rust
use bevy::window::ExitCondition;

App::new()
    .add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window::default()),
        exit_condition: ExitCondition::DontExit,  // Manual exit only
        ..default()
    }))
```

Options:
- `ExitCondition::OnPrimaryClosed` - Exit when primary window closes (default)
- `ExitCondition::OnAllClosed` - Exit when all windows close
- `ExitCondition::DontExit` - Never auto-exit, requires manual `AppExit`

## Testing Shutdown Behavior

### Example Test Setup

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::AppExit;

    #[test]
    fn test_save_on_shutdown() {
        let mut app = App::new();
        app.add_systems(Last, save_on_shutdown);

        // Trigger exit
        app.world_mut().send_message(AppExit::Success);

        // Run one update (processes exit)
        app.update();

        // Verify save occurred
        assert!(std::path::Path::new("test_save.bin").exists());
    }
}
```

## Performance Considerations

### Blocking I/O Budget

Acceptable shutdown I/O time depends on context:
- **Desktop game**: 1-2 seconds acceptable
- **Server**: 5-10 seconds acceptable (can show "Shutting down..." message)
- **Mobile**: <500ms (OS may kill process)

### Async Alternative (Future Enhancement)

For very large saves, consider async I/O:

```rust
use bevy::tasks::{AsyncComputeTaskPool, Task};

#[derive(Resource)]
struct SaveTask(Task<std::io::Result<()>>);

fn start_save_on_shutdown(
    mut commands: Commands,
    mut exit_reader: MessageReader<AppExit>,
    data: Res<GameData>,
) {
    if !exit_reader.is_empty() {
        exit_reader.clear();

        let thread_pool = AsyncComputeTaskPool::get();
        let data = data.clone();

        let task = thread_pool.spawn(async move {
            save_to_file_async(&data).await
        });

        commands.insert_resource(SaveTask(task));
    }
}

fn wait_for_save(
    mut commands: Commands,
    mut task: Option<ResMut<SaveTask>>,
    mut app_exit: MessageWriter<AppExit>,
) {
    if let Some(mut task) = task {
        if let Some(result) = block_on(poll_once(&mut task.0)) {
            match result {
                Ok(_) => info!("Save complete"),
                Err(e) => error!("Save failed: {e}"),
            }
            commands.remove_resource::<SaveTask>();
            app_exit.write(AppExit::Success);
        }
    }
}
```

**Note**: Requires deferring actual exit until save completes.

## Summary Table

| Aspect | Recommendation | Avoid |
|--------|---------------|-------|
| **Trigger exit** | `MessageWriter<AppExit>` | `std::process::exit()` |
| **Detect exit** | `MessageReader<AppExit>` | `OnExit` states |
| **Schedule** | `Last` | `OnExit`, `StateTransition` |
| **File I/O location** | Main world systems | Render world systems |
| **File write pattern** | Temp file + atomic rename | Direct overwrite |
| **Error handling** | Log and continue exit | Panic or abort |
| **Critical cleanup** | `Drop` impl | Exit-only systems |
| **Long saves** | Blocking OK on shutdown | Not OK during gameplay |

## Code References

- `git/bevy/crates/bevy_app/src/app.rs:1475-1506` - AppExit definition
- `git/bevy/crates/bevy_app/src/terminal_ctrl_c_handler.rs:41-74` - Ctrl+C handling
- `git/bevy/crates/bevy_winit/src/system.rs:241-283` - Window cleanup on exit
- `git/bevy/crates/bevy_window/src/system.rs:13-26` - Auto-exit patterns
- `git/bevy/examples/app/headless_renderer.rs:473-548` - File I/O on exit
- `git/bevy/examples/games/game_menu.rs:686-700` - User-triggered exit
- `git/bevy/crates/bevy_app/src/main_schedule.rs:12-38` - Schedule order

## Related Documentation

- Project: `doc/research/2026-01-17-voxel-world-save-load.md` - Save/load implementation context
- Bevy docs: [App lifecycle](https://docs.rs/bevy/latest/bevy/app/struct.App.html)
- Bevy docs: [Schedules](https://docs.rs/bevy/latest/bevy/ecs/schedule/index.html)
