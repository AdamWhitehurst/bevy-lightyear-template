pub mod gameplay;
pub mod map;
pub mod network;
pub mod world_object;

use avian3d::prelude::PhysicsDebugPlugin;
use bevy::prelude::*;
use bevy::time::Real;
use gameplay::ClientGameplayPlugin;
use lightyear::prelude::client::*;
use lightyear::prelude::{InputTimeline, IsSynced, PredictionMetrics};
use map::ClientMapPlugin;
use network::{ClientNetworkConfig, ClientNetworkPlugin};
use protocol::*;
use render::RenderPlugin;
use std::time::Duration;
use tracy_client::plot;
use ui::{UiClientConfig, UiPlugin};
use voxel_map_engine::prelude::VoxelChunk;

fn main() {
    let client_id = parse_client_id();

    let network_config = ClientNetworkConfig {
        client_id,
        ..Default::default()
    };

    // Create UI config from network config to keep them in sync
    let ui_config = UiClientConfig {
        server_addr: network_config.server_addr,
        client_id: network_config.client_id,
        protocol_id: network_config.protocol_id,
        private_key: network_config.private_key,
    };

    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin {
            file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets").to_string(),
            ..default()
        }))
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(SharedGameplayPlugin)
        .add_plugins(ClientNetworkPlugin {
            config: network_config,
        })
        .insert_resource(ui_config) // Override default UiClientConfig
        .add_plugins(ClientGameplayPlugin)
        .add_plugins(ClientMapPlugin)
        .add_plugins(RenderPlugin)
        .add_plugins(UiPlugin)
        .add_plugins(PhysicsDebugPlugin::default())
        .init_resource::<FixedStepCounter>()
        .init_resource::<PrevRollbackMetrics>()
        .add_systems(FixedUpdate, count_fixed_steps)
        .add_systems(
            Last,
            (
                plot_frame_diagnostics,
                plot_rollback_diagnostics,
                plot_input_sync_status,
            ),
        )
        .run();
}

/// Tracks how many FixedUpdate steps ran per frame.
#[derive(Resource, Default)]
struct FixedStepCounter {
    steps: u32,
}

/// Stores previous cumulative rollback metrics to compute per-frame deltas.
#[derive(Resource, Default)]
struct PrevRollbackMetrics {
    rollbacks: u32,
    rollback_ticks: u32,
}

fn count_fixed_steps(mut counter: ResMut<FixedStepCounter>) {
    counter.steps += 1;
}

/// Plots client frame time and FixedUpdate step count to tracy.
fn plot_frame_diagnostics(time: Res<Time<Real>>, mut counter: ResMut<FixedStepCounter>) {
    let frame_ms = time.delta().as_secs_f64() * 1000.0;
    let steps = counter.steps;
    plot!("cli_frame_ms", frame_ms);
    plot!("cli_fixed_steps", steps as f64);
    plot!(
        "cli_max_delta_hit",
        if frame_ms > 250.0 { 1.0 } else { 0.0 }
    );
    counter.steps = 0;
}

/// Plots per-frame rollback deltas and chunk collider insertions to tracy.
fn plot_rollback_diagnostics(
    metrics: Res<PredictionMetrics>,
    mut prev: ResMut<PrevRollbackMetrics>,
    new_colliders: Query<(), (With<VoxelChunk>, Added<avian3d::prelude::Collider>)>,
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
/// 0 = inputs NOT being sent, 1 = inputs being sent normally.
fn plot_input_sync_status(query: Query<Has<IsSynced<InputTimeline>>, With<InputTimeline>>) {
    let synced = query.iter().any(|has| has);
    plot!("cli_input_timeline_synced", if synced { 1.0 } else { 0.0 });
}

fn parse_client_id() -> u64 {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "-c" || args[i] == "--client-id" {
            if let Some(id_str) = args.get(i + 1) {
                return id_str.parse().expect("Invalid client ID");
            }
        }
    }
    0
}
