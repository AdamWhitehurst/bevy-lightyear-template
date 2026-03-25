pub mod gameplay;
pub mod map;
pub mod network;
pub mod persistence;
pub mod world_object;

use bevy::prelude::*;
use bevy::time::Real;
use gameplay::ServerGameplayPlugin;
use lightyear::prelude::input::leafwing::LeafwingBuffer;
use lightyear::prelude::*;
use map::ServerMapPlugin;
use network::ServerNetworkPlugin;
use protocol::*;
use std::time::Duration;
use tracy_client::plot;

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(bevy::state::app::StatesPlugin)
        .add_plugins(bevy::log::LogPlugin::default())
        .add_plugins(bevy::asset::AssetPlugin {
            file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets").to_string(),
            ..default()
        })
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::scene::ScenePlugin)
        // Register asset resources for voxel world mesh generation
        .add_message::<bevy::asset::AssetEvent<bevy::prelude::Mesh>>()
        .init_asset::<bevy::prelude::Mesh>()
        .init_asset::<bevy::pbr::StandardMaterial>()
        .init_asset::<bevy::shader::Shader>()
        .add_message::<bevy::asset::AssetEvent<bevy::shader::Shader>>()
        .init_asset::<bevy::image::Image>()
        .add_message::<bevy::asset::AssetEvent<bevy::image::Image>>()
        .add_plugins(lightyear::prelude::server::ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(SharedGameplayPlugin)
        .add_plugins(ServerNetworkPlugin::default())
        .add_plugins(ServerGameplayPlugin)
        .add_plugins(ServerMapPlugin)
        .init_resource::<FixedStepCounter>()
        .add_systems(FixedUpdate, count_fixed_steps)
        .add_systems(Last, (plot_frame_diagnostics, plot_input_buffer_status))
        .run();
}

/// Tracks how many FixedUpdate steps ran per frame.
#[derive(Resource, Default)]
struct FixedStepCounter {
    steps: u32,
}

fn count_fixed_steps(mut counter: ResMut<FixedStepCounter>) {
    counter.steps += 1;
}

/// Plots server frame time and FixedUpdate step count to tracy.
fn plot_frame_diagnostics(time: Res<Time<Real>>, mut counter: ResMut<FixedStepCounter>) {
    let frame_ms = time.delta().as_secs_f64() * 1000.0;
    let steps = counter.steps;
    plot!("srv_frame_ms", frame_ms);
    plot!("srv_fixed_steps", steps as f64);
    plot!(
        "srv_max_delta_hit",
        if frame_ms > 250.0 { 1.0 } else { 0.0 }
    );
    counter.steps = 0;
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
                    // Wrapping subtraction: positive = server ahead of buffer start
                    let offset = server_tick - start;
                    plot!("srv_tick_ahead_of_buffer", offset as f64);
                    let buf_end_offset = offset as i32 - buffer.buffer.len() as i32;
                    // Positive = server past buffer end (inputs too old = BUG)
                    // Negative = server within or behind buffer (normal)
                    plot!("srv_tick_past_buffer_end", buf_end_offset as f64);
                }
            }
            None => {
                plot!("srv_has_input_buffer", 0.0);
            }
        }
    }
}
