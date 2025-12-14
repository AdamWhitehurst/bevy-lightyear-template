pub mod network;
pub mod gameplay;

pub use lightyear::prelude::Authentication;
pub use lightyear::netcode::{Key, NetcodeClient};
pub use lightyear::prelude::client::NetcodeConfig;
pub use network::{ClientNetworkConfig, ClientTransport};
