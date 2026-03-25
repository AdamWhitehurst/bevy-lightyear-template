//! Client-specific tracy diagnostics (rollback metrics, input sync, chunk colliders).

use avian3d::prelude::Collider;
use bevy::prelude::*;
use lightyear::prelude::{InputTimeline, IsSynced, PredictionMetrics};
use tracy_client::plot;
use voxel_map_engine::prelude::VoxelChunk;

/// Client-specific tracy diagnostics.
pub struct ClientDiagnosticsPlugin;

impl Plugin for ClientDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PrevRollbackMetrics>()
            .add_systems(Last, (plot_rollback_diagnostics, plot_input_sync_status));
    }
}

/// Stores previous cumulative rollback metrics to compute per-frame deltas.
#[derive(Resource, Default)]
struct PrevRollbackMetrics {
    rollbacks: u32,
    rollback_ticks: u32,
}

/// Plots per-frame rollback deltas and chunk collider insertions.
fn plot_rollback_diagnostics(
    metrics: Res<PredictionMetrics>,
    mut prev: ResMut<PrevRollbackMetrics>,
    new_colliders: Query<(), (With<VoxelChunk>, Added<Collider>)>,
) {
    let rollbacks_this_frame = metrics.rollbacks.saturating_sub(prev.rollbacks);
    let rollback_ticks_this_frame = metrics.rollback_ticks.saturating_sub(prev.rollback_ticks);
    prev.rollbacks = metrics.rollbacks;
    prev.rollback_ticks = metrics.rollback_ticks;

    plot!("cli_rollbacks", rollbacks_this_frame as f64);
    plot!("cli_rollback_ticks", rollback_ticks_this_frame as f64);
    plot!(
        "cli_chunk_colliders_added",
        new_colliders.iter().count() as f64
    );
}

/// Plots whether the input timeline is synced (required for input delivery).
fn plot_input_sync_status(query: Query<Has<IsSynced<InputTimeline>>, With<InputTimeline>>) {
    let synced = query.iter().any(|has| has);
    plot!("cli_input_timeline_synced", if synced { 1.0 } else { 0.0 });
}
