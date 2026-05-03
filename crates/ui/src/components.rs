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

/// Marker for the map switch toggle button in in-game HUD
#[derive(Component)]
pub struct MapSwitchButton;

#[derive(Component)]
pub struct GenerateButton;

#[derive(Component)]
pub struct ImportButton;

#[derive(Component)]
pub struct UnlockButton;

#[derive(Component)]
pub struct PassphraseInput;

#[derive(Component)]
pub struct NsecInput;

#[derive(Component)]
pub struct ServerListContainer;

#[derive(Component)]
pub struct ServerListEntryButton(pub nostr_client::announcement::ServerListEntry);
