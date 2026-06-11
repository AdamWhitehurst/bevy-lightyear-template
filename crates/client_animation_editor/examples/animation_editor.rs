use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin {
            file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets").to_string(),
            ..default()
        }))
        .add_plugins(protocol::app_state::AppStatePlugin)
        .add_plugins(bevy_egui::EguiPlugin::default())
        .add_plugins(client_animation_editor::AnimationEditorPlugin)
        .run();
}
