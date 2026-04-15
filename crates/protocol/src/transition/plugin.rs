use bevy::prelude::*;
use lightyear::prelude::*;

use super::types::*;

pub struct TransitionPlugin;

impl Plugin for TransitionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientTransitionState>();
        app.register_type::<TransitionPhase>();

        // Register new message. Channel is specified at send time
        // (sender.send::<MapChannel>(...)), not at registration.
        // Existing MapTransitionStart/Ready/End registration stays
        // in ProtocolPlugin::build (protocol/src/lib.rs:133-147).
        app.register_message::<MapTransitionEntity>()
            .add_direction(NetworkDirection::ServerToClient);
    }
}
