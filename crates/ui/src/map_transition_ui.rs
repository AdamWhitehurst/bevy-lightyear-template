use bevy::prelude::*;

use crate::state::MapTransitionState;

pub struct MapTransitionUiPlugin;

impl Plugin for MapTransitionUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(MapTransitionState::Transitioning),
            setup_transition_overlay,
        );
    }
}

/// Marker for the map transition loading overlay.
#[derive(Component)]
pub struct MapTransitionOverlay;

fn setup_transition_overlay(mut commands: Commands) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                position_type: PositionType::Absolute,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            MapTransitionOverlay,
            DespawnOnExit(MapTransitionState::Transitioning),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Loading..."),
                TextFont {
                    font_size: 48.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}
