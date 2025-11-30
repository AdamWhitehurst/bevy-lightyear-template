use bevy::prelude::*;

/// Marker for Connect button in main menu
#[derive(Component)]
pub struct ConnectButton;

/// Marker for Quit button (appears in main menu and in-game)
#[derive(Component)]
pub struct QuitButton;

/// Marker for Main Menu button in in-game UI
#[derive(Component)]
pub struct MainMenuButton;

/// Marker for Cancel button in connecting screen
#[derive(Component)]
pub struct CancelButton;
