use bevy::prelude::*;
use protocol::AppStatePlugin;
use render::RenderPlugin;

#[test]
fn test_render_plugin_adds_camera() {
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins
            .set(bevy::window::WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
            .set(bevy::render::RenderPlugin {
                render_creation: bevy::render::settings::RenderCreation::Automatic(
                    bevy::render::settings::WgpuSettings {
                        backends: None,
                        ..default()
                    },
                ),
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .add(bevy::app::ScheduleRunnerPlugin::default()),
        AppStatePlugin,
        RenderPlugin,
    ));

    // Run only Startup schedule to avoid needing lightyear runtime resources
    app.world_mut().run_schedule(Startup);

    let mut query = app.world_mut().query::<&Camera3d>();
    let result = query.single(app.world());
    assert!(result.is_ok(), "Exactly one camera should exist");
}
