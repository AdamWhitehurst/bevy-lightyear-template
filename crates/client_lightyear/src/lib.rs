//! Generic native+WASM WebTransport client setup.
mod connection;
mod netcode;
mod webtransport;

pub use connection::{ClientNetworkConfig, ClientNetworkPlugin};
