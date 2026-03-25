//! Server-specific tracy diagnostics (input buffer status).

use bevy::prelude::*;
use lightyear::prelude::input::leafwing::LeafwingBuffer;
use lightyear::prelude::*;
use protocol::{CharacterMarker, PlayerActions};
use tracy_client::plot;

/// Server-specific tracy diagnostics.
pub struct ServerDiagnosticsPlugin;

impl Plugin for ServerDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Last, plot_input_buffer_status);
    }
}

/// Plots server tick vs input buffer tick range to diagnose tick misalignment.
fn plot_input_buffer_status(
    timeline: Res<LocalTimeline>,
    query: Query<(Option<&LeafwingBuffer<PlayerActions>>, &CharacterMarker), With<ControlledBy>>,
) {
    let server_tick = timeline.tick();
    for (buffer_opt, _) in &query {
        match buffer_opt {
            Some(buffer) => {
                plot!("srv_has_input_buffer", 1.0);
                plot!("srv_input_buffer_len", buffer.buffer.len() as f64);
                if let Some(start) = buffer.start_tick {
                    let offset = server_tick - start;
                    plot!("srv_tick_ahead_of_buffer", offset as f64);
                    let buf_end_offset = offset as i32 - buffer.buffer.len() as i32;
                    plot!("srv_tick_past_buffer_end", buf_end_offset as f64);
                }
            }
            None => {
                plot!("srv_has_input_buffer", 0.0);
            }
        }
    }
}
