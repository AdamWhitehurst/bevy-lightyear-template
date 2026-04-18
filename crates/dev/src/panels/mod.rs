//! Per-panel modules. Each is gated by its own Cargo feature so disabled
//! panels pay zero compile + zero runtime cost.

#[cfg(feature = "world-inspector")]
pub mod world_inspector;

#[cfg(feature = "spawn-panel")]
pub mod spawn;

#[cfg(feature = "netviz")]
pub mod netviz;

#[cfg(feature = "chunk-debug")]
pub mod chunk_debug;

#[cfg(feature = "ability-editor")]
pub mod ability_editor;

#[cfg(feature = "world-object-editor")]
pub mod world_object_editor;
