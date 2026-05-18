use bevy::prelude::*;

/// Ordered client input routing stages.
#[derive(SystemSet, Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ClientInputSet {
    Capture,
    WriteTransport,
    ProduceLocalCommands,
    Consume,
}
