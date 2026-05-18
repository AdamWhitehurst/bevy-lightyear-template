//! Tracy diagnostics for gameplay systems.
//!
//! All `plot!` calls are no-ops when `tracy-client/enable` is not active.

use bevy::prelude::*;
use bevy::time::Real;
use leafwing_input_manager::prelude::ActionState;
use tracy_client::plot;

use crate::NetworkedPlayerActions;

/// Shared tracy diagnostics registered by both client and server.
pub struct SharedDiagnosticsPlugin;

impl Plugin for SharedDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FixedStepCounter>()
            .add_systems(FixedUpdate, count_fixed_steps)
            .add_systems(Last, plot_frame_diagnostics);
    }
}

/// Tracks how many FixedUpdate steps ran per frame.
#[derive(Resource, Default)]
struct FixedStepCounter {
    steps: u32,
}

fn count_fixed_steps(mut counter: ResMut<FixedStepCounter>) {
    counter.steps += 1;
}

/// Plots frame time and FixedUpdate step count to tracy.
fn plot_frame_diagnostics(time: Res<Time<Real>>, mut counter: ResMut<FixedStepCounter>) {
    let frame_ms = time.delta().as_secs_f64() * 1000.0;
    let steps = counter.steps;
    plot!("frame_ms", frame_ms);
    plot!("fixed_steps", steps as f64);
    plot!("max_delta_hit", if frame_ms > 250.0 { 1.0 } else { 0.0 });
    counter.steps = 0;
}

/// Plots per-tick input state from an `ActionState`.
///
/// Called by client and server diagnostics plugins with appropriate entity
/// filters — the client must filter to `With<Predicted>` to avoid reading
/// the Confirmed entity's stale replicated ActionState.
pub fn plot_action_state(action_state: &ActionState<NetworkedPlayerActions>) {
    plot!(
        "move_input_magnitude",
        action_state
            .axis_pair(&NetworkedPlayerActions::Move)
            .length() as f64
    );
    plot!(
        "any_ability_pressed",
        if action_state.pressed(&NetworkedPlayerActions::Ability1)
            || action_state.pressed(&NetworkedPlayerActions::Ability2)
            || action_state.pressed(&NetworkedPlayerActions::Ability3)
            || action_state.pressed(&NetworkedPlayerActions::Ability4)
        {
            1.0
        } else {
            0.0
        }
    );
}
