use bevy::prelude::*;

/// Client application state
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, States)]
pub enum ClientState {
    /// Startup loading gate - not interactive until global app readiness completes
    #[default]
    Loading,
    /// Main menu - not connected to server
    MainMenu,
    /// Connecting to server - loading screen
    Connecting,
    /// Connected and in-game
    InGame,
}

/// Sub-state for map transition flow while in-game
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, SubStates)]
#[source(ClientState = ClientState::InGame)]
pub enum MapTransitionState {
    #[default]
    Playing,
    Transitioning,
}
