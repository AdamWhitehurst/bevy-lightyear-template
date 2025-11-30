pub mod state;
pub mod components;

use bevy::prelude::*;
use bevy::ecs::message::MessageWriter;
use lightyear::prelude::client::*;
pub use state::ClientState;
pub use components::*;

/// Plugin that manages UI and client state
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        // Initialize state management
        app.init_state::<ClientState>();

        // State transition systems
        app.add_systems(OnEnter(ClientState::Connecting), on_entering_connecting_state);
        app.add_observer(on_client_disconnected);
        app.add_observer(on_client_connected);

        // Main menu
        app.add_systems(OnEnter(ClientState::MainMenu), setup_main_menu);
        app.add_systems(Update, main_menu_button_interaction.run_if(in_state(ClientState::MainMenu)));

        // Connecting screen
        app.add_systems(OnEnter(ClientState::Connecting), setup_connecting_screen);
        app.add_systems(Update, connecting_screen_interaction.run_if(in_state(ClientState::Connecting)));

        // In-game HUD
        app.add_systems(OnEnter(ClientState::InGame), setup_ingame_hud);
        app.add_systems(Update, ingame_button_interaction.run_if(in_state(ClientState::InGame)));

        info!("UiPlugin initialized");
    }
}

fn on_entering_connecting_state(
    mut commands: Commands,
    client_query: Query<Entity, With<Client>>,
) {
    info!("Entering Connecting state, triggering connection...");
    let client_entity = client_query.single().expect("Client entity should exist");
    commands.trigger(Connect {
        entity: client_entity,
    });
}

fn on_client_disconnected(
    _trigger: On<Add, Disconnected>,
    mut next_state: ResMut<NextState<ClientState>>,
    current_state: Res<State<ClientState>>,
) {
    // Only transition if not already in MainMenu
    if *current_state.get() != ClientState::MainMenu {
        info!("Client disconnected, returning to main menu");
        next_state.set(ClientState::MainMenu);
    }
}

fn on_client_connected(
    _trigger: On<Add, Connected>,
    mut next_state: ResMut<NextState<ClientState>>,
) {
    info!("Client connected, transitioning to InGame state");
    next_state.set(ClientState::InGame);
}

fn setup_main_menu(mut commands: Commands) {
    info!("Setting up main menu UI");

    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(20.0),
            ..default()
        },
        BackgroundColor(Color::srgb(0.1, 0.1, 0.1)),
        DespawnOnExit(ClientState::MainMenu),
    ))
    .with_children(|parent| {
        // Title
        parent.spawn((
            Text::new("Lightyear Client"),
            TextFont {
                font_size: 60.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));

        // Connect Button
        parent.spawn((
            Button,
            Node {
                width: Val::Px(200.0),
                height: Val::Px(65.0),
                border: UiRect::all(Val::Px(5.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(Color::WHITE),
            BackgroundColor(Color::srgb(0.2, 0.2, 0.2)),
            ConnectButton,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Connect"),
                TextFont {
                    font_size: 33.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });

        // Quit Button
        parent.spawn((
            Button,
            Node {
                width: Val::Px(200.0),
                height: Val::Px(65.0),
                border: UiRect::all(Val::Px(5.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(Color::WHITE),
            BackgroundColor(Color::srgb(0.2, 0.2, 0.2)),
            QuitButton,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Quit"),
                TextFont {
                    font_size: 33.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
    });
}

fn main_menu_button_interaction(
    mut next_state: ResMut<NextState<ClientState>>,
    mut exit_writer: MessageWriter<AppExit>,
    connect_query: Query<&Interaction, (Changed<Interaction>, With<ConnectButton>)>,
    quit_query: Query<&Interaction, (Changed<Interaction>, With<QuitButton>)>,
) {
    // Handle Connect button
    for interaction in connect_query.iter() {
        if *interaction == Interaction::Pressed {
            info!("Connect button pressed");
            next_state.set(ClientState::Connecting);
        }
    }

    // Handle Quit button
    for interaction in quit_query.iter() {
        if *interaction == Interaction::Pressed {
            info!("Quit button pressed");
            exit_writer.write(AppExit::Success);
        }
    }
}

fn setup_connecting_screen(mut commands: Commands) {
    info!("Setting up connecting screen UI");

    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(20.0),
            ..default()
        },
        BackgroundColor(Color::srgb(0.1, 0.1, 0.1)),
        DespawnOnExit(ClientState::Connecting),
    ))
    .with_children(|parent| {
        // Connecting message
        parent.spawn((
            Text::new("Connecting to server..."),
            TextFont {
                font_size: 40.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));

        // Cancel Button
        parent.spawn((
            Button,
            Node {
                width: Val::Px(200.0),
                height: Val::Px(65.0),
                border: UiRect::all(Val::Px(5.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(Color::WHITE),
            BackgroundColor(Color::srgb(0.2, 0.2, 0.2)),
            CancelButton,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Cancel"),
                TextFont {
                    font_size: 33.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
    });
}

fn connecting_screen_interaction(
    mut commands: Commands,
    mut next_state: ResMut<NextState<ClientState>>,
    client_query: Query<Entity, With<Client>>,
    cancel_query: Query<&Interaction, (Changed<Interaction>, With<CancelButton>)>,
) {
    for interaction in cancel_query.iter() {
        if *interaction == Interaction::Pressed {
            info!("Cancel button pressed, disconnecting...");

            let client_entity = client_query.single().expect("Client entity should exist");
            // Trigger disconnection
            commands.trigger(Disconnect {
                entity: client_entity,
            });

            // Return to main menu (observer will also handle this, but explicit is clearer)
            next_state.set(ClientState::MainMenu);
        }
    }
}

fn setup_ingame_hud(mut commands: Commands) {
    info!("Setting up in-game HUD");

    // Top-right corner HUD
    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::End,
            align_items: AlignItems::Start,
            padding: UiRect::all(Val::Px(20.0)),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(10.0),
            ..default()
        },
        DespawnOnExit(ClientState::InGame),
    ))
    .with_children(|parent| {
        // Main Menu Button
        parent.spawn((
            Button,
            Node {
                width: Val::Px(150.0),
                height: Val::Px(50.0),
                border: UiRect::all(Val::Px(3.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(Color::WHITE),
            BackgroundColor(Color::srgba(0.2, 0.2, 0.2, 0.8)),
            MainMenuButton,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Main Menu"),
                TextFont {
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });

        // Quit Button
        parent.spawn((
            Button,
            Node {
                width: Val::Px(150.0),
                height: Val::Px(50.0),
                border: UiRect::all(Val::Px(3.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(Color::WHITE),
            BackgroundColor(Color::srgba(0.2, 0.2, 0.2, 0.8)),
            QuitButton,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Quit"),
                TextFont {
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
    });
}

fn ingame_button_interaction(
    mut commands: Commands,
    mut next_state: ResMut<NextState<ClientState>>,
    mut exit_writer: MessageWriter<AppExit>,
    client_query: Query<Entity, With<Client>>,
    main_menu_query: Query<&Interaction, (Changed<Interaction>, With<MainMenuButton>)>,
    quit_query: Query<&Interaction, (Changed<Interaction>, With<QuitButton>, Without<MainMenuButton>)>,
) {
    // Handle Main Menu button
    for interaction in main_menu_query.iter() {
        if *interaction == Interaction::Pressed {
            info!("Main Menu button pressed, disconnecting...");

            let client_entity = client_query.single().expect("Client entity should exist");
            // Trigger disconnection
            commands.trigger(Disconnect {
                entity: client_entity,
            });

            // Return to main menu (observer will also handle this)
            next_state.set(ClientState::MainMenu);
        }
    }

    // Handle Quit button
    for interaction in quit_query.iter() {
        if *interaction == Interaction::Pressed {
            info!("Quit button pressed");
            exit_writer.write(AppExit::Success);
        }
    }
}
