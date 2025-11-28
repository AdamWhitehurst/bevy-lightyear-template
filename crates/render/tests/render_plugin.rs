use bevy::prelude::*;
use render::RenderPlugin;

#[test]
fn test_render_plugin_adds_camera() {
    let mut app = App::new();
    app.add_plugins(RenderPlugin);
    app.update();

    let mut query = app.world_mut().query::<&Camera3d>();
    let result = query.single(app.world());
    assert!(result.is_ok(), "Exactly one camera should exist");
}
