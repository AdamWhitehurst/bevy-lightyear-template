use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;
use bevy_ui_text_input::{TextInputBuffer, TextInputMode, TextInputNode, TextInputPrompt};

pub const SCREEN_BG: Color = Color::srgb(0.1, 0.1, 0.1);
pub const BUTTON_BG: Color = Color::srgb(0.2, 0.2, 0.2);
pub const TEXT_COLOR: Color = Color::WHITE;
pub const BUTTON_SIZE: Vec2 = Vec2::new(240.0, 65.0);

pub fn spawn_button<M: Component>(parent: &mut ChildSpawnerCommands, label: &str, marker: M) {
    parent
        .spawn((
            Button,
            Node {
                width: Val::Px(BUTTON_SIZE.x),
                height: Val::Px(BUTTON_SIZE.y),
                border: UiRect::all(Val::Px(5.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(TEXT_COLOR),
            BackgroundColor(BUTTON_BG),
            marker,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(label),
                TextFont {
                    font_size: 33.0,
                    ..default()
                },
                TextColor(TEXT_COLOR),
            ));
        });
}

pub fn spawn_text_input<M: Component>(
    parent: &mut ChildSpawnerCommands,
    placeholder: &str,
    marker: M,
    _password: bool,
) {
    parent.spawn((
        TextInputNode {
            mode: TextInputMode::SingleLine,
            ..default()
        },
        TextInputBuffer::default(),
        TextInputPrompt::new(placeholder),
        Node {
            width: Val::Px(520.0),
            height: Val::Px(48.0),
            ..default()
        },
        marker,
    ));
}
