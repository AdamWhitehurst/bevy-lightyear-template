//! Per-panel modules. Each is gated by its own Cargo feature so disabled
//! panels pay zero compile + zero runtime cost.

#[cfg(feature = "world-inspector")]
pub mod world_inspector;

#[cfg(feature = "spawn-panel")]
pub mod spawn;

#[cfg(feature = "spawn-panel")]
pub mod terrain;
