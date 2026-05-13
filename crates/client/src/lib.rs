pub mod auth;
pub mod gameplay;
pub mod map;
pub mod persistence;
pub mod transition;
pub mod world_object;

pub use client_lightyear::ClientNetworkConfig;
pub use lightyear::netcode::{Key, NetcodeClient};
pub use lightyear::prelude::Authentication;
pub use lightyear::prelude::client::NetcodeConfig;
pub use map::{PlacementTarget, current_placement_target};
