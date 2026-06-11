use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use bevy_egui::egui;
use protocol::CharacterMarker;

use crate::state::EditorState;

/// Locomotion audition input: horizontal speed written into the rig's velocity.
#[derive(Resource, Default)]
pub struct AuditionState {
    pub speed: f32,
}

/// Speed slider drawn inside the transport bar. speed = 0 auditions the edited clip over
/// the idle base (solo); higher speeds blend walk/run underneath via the REAL
/// `update_locomotion_blend_weights` — the editor sets the input, never forks blend logic.
pub fn draw_audition_controls(
    ui: &mut egui::Ui,
    audition: &mut AuditionState,
    state: &EditorState,
) {
    let max_threshold = state
        .working_set
        .locomotion
        .entries
        .last()
        .map(|e| e.speed_threshold)
        .unwrap_or(0.0);
    ui.add(
        egui::Slider::new(&mut audition.speed, 0.0..=(max_threshold * 1.25).max(1.0)).text("speed"),
    );
}

/// Writes the audition speed into the editor rig's `LinearVelocity` — the same component
/// the in-game blend reads (`velocity.xz().length()` feeds `compute_blend_weights`).
pub fn set_audition_velocity(
    audition: Res<AuditionState>,
    mut rigs: Query<&mut LinearVelocity, With<CharacterMarker>>,
) {
    let Ok(mut velocity) = rigs.single_mut() else {
        trace!("editor rig not spawned; audition idle");
        return;
    };
    velocity.0 = Vec3::new(audition.speed, 0.0, 0.0);
}
