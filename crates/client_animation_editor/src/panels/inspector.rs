use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use sprite_rig::asset::SpriteAnimAsset;

use crate::eval::{rotation_keys, sample_scalar, sample_vec2, vec2_keys};
use crate::state::{Channel, EditorState, Selection};

/// Values closer than this at t=0 vs t=duration are considered loop-continuous.
const LOOP_EPSILON: f32 = 1e-4;

/// Right-side inspector: value-at-playhead for the selected channel (via `eval`), the
/// selected key's exact time/value, the selected event's time + editable name, and the
/// loop-continuity warning for looping clips.
pub fn draw_inspector(mut contexts: EguiContexts, mut state: ResMut<EditorState>) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("egui context not ready; skipping inspector frame");
        return;
    };
    egui::SidePanel::right("inspector")
        .resizable(true)
        .default_width(200.0)
        .show(ctx, |ui| {
            ui.heading(&state.working.name);
            ui.label(format!("duration: {:.3}s", state.working.duration));
            ui.label(format!("looping: {}", state.working.looping));
            ui.separator();

            match state.selection.clone() {
                Selection::Key { bone, channel, idx } => {
                    draw_key_details(ui, &state, &bone, channel, idx);
                }
                Selection::Event(idx) => {
                    draw_event_details(ui, &mut state, idx);
                }
                Selection::None => {
                    ui.weak("nothing selected");
                }
            }

            if let Some(warning) = loop_continuity_warning(&state.working) {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(230, 160, 60), warning);
            }
        });
}

/// Selected key's exact time/value plus the channel's sampled value at the playhead.
fn draw_key_details(
    ui: &mut egui::Ui,
    state: &EditorState,
    bone: &str,
    channel: Channel,
    idx: usize,
) {
    let Some(timeline) = state.working.bone_timelines.get(bone) else {
        debug_assert!(false, "selected bone '{bone}' missing from working clip");
        return;
    };
    ui.label(format!("{bone}.{}", channel_name(channel)));
    match channel {
        Channel::Rotation => {
            let keys = rotation_keys(timeline);
            let Some((time, value, curve)) = keys.get(idx) else {
                debug_assert!(false, "selected rotation key {idx} out of bounds");
                return;
            };
            ui.label(format!("key t: {time:.4}s"));
            ui.label(format!("key value: {value:.3}°"));
            ui.label(format!("curve: {curve:?}"));
            ui.separator();
            ui.label(format!(
                "at playhead: {:.3}°",
                sample_scalar(&keys, state.playhead)
            ));
        }
        Channel::Translation | Channel::Scale => {
            let keys = vec2_keys(timeline, channel);
            let Some((time, value, curve)) = keys.get(idx) else {
                debug_assert!(false, "selected {channel:?} key {idx} out of bounds");
                return;
            };
            ui.label(format!("key t: {time:.4}s"));
            ui.label(format!("key value: ({:.3}, {:.3})", value.x, value.y));
            ui.label(format!("curve: {curve:?}"));
            let v = sample_vec2(&keys, state.playhead);
            ui.separator();
            ui.label(format!("at playhead: ({:.3}, {:.3})", v.x, v.y));
        }
    }
}

/// Selected animation event's time plus an editable name field. A rename marks the clip
/// dirty so the rebake keeps the baked clip's events in sync.
fn draw_event_details(ui: &mut egui::Ui, state: &mut EditorState, idx: usize) {
    let Some(event) = state.working.events.get(idx) else {
        debug_assert!(false, "selected event {idx} out of bounds");
        return;
    };
    ui.label(format!("event t: {:.4}s", event.time));
    let mut name = event.name.clone();
    ui.horizontal(|ui| {
        ui.label("name:");
        if ui.text_edit_singleline(&mut name).changed() {
            state.working.events[idx].name = name;
            state.clip_dirty = true;
        }
    });
}

/// For a `looping` clip, flags any channel whose value at t=0 differs from t=duration —
/// a discontinuity that will pop on loop. Returns a human-readable message or `None`.
pub fn loop_continuity_warning(asset: &SpriteAnimAsset) -> Option<String> {
    if !asset.looping {
        return None;
    }
    let mut discontinuous: Vec<String> = Vec::new();
    let mut bones: Vec<&String> = asset.bone_timelines.keys().collect();
    bones.sort();
    for bone in bones {
        let timeline = &asset.bone_timelines[bone];
        let rotation = rotation_keys(timeline);
        if !rotation.is_empty() {
            let start = sample_scalar(&rotation, 0.0);
            let end = sample_scalar(&rotation, asset.duration);
            if (start - end).abs() > LOOP_EPSILON {
                discontinuous.push(format!("{bone}.rotation"));
            }
        }
        for (name, channel) in [
            ("translation", Channel::Translation),
            ("scale", Channel::Scale),
        ] {
            let keys = vec2_keys(timeline, channel);
            if keys.is_empty() {
                continue; // channel not authored for this bone — nothing to compare
            }
            let start = sample_vec2(&keys, 0.0);
            let end = sample_vec2(&keys, asset.duration);
            if start.distance(end) > LOOP_EPSILON {
                discontinuous.push(format!("{bone}.{name}"));
            }
        }
    }
    (!discontinuous.is_empty()).then(|| {
        format!(
            "loop discontinuity (t=0 ≠ t=duration): {}",
            discontinuous.join(", ")
        )
    })
}

fn channel_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Rotation => "rotation",
        Channel::Translation => "translation",
        Channel::Scale => "scale",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sprite_rig::asset::{BoneTimeline, CurveType, RotationKeyframe};
    use std::collections::HashMap;

    fn clip(looping: bool, end_value: f32) -> SpriteAnimAsset {
        let mut bone_timelines = HashMap::new();
        bone_timelines.insert(
            "root".to_string(),
            BoneTimeline {
                rotation: vec![
                    RotationKeyframe {
                        time: 0.0,
                        value: 0.0,
                        curve: CurveType::Linear,
                    },
                    RotationKeyframe {
                        time: 1.0,
                        value: end_value,
                        curve: CurveType::Linear,
                    },
                ],
                ..Default::default()
            },
        );
        SpriteAnimAsset {
            name: "test".to_string(),
            duration: 1.0,
            looping,
            bone_timelines,
            events: vec![],
        }
    }

    #[test]
    fn mismatched_endpoints_warn() {
        let warning = loop_continuity_warning(&clip(true, 90.0));
        assert!(warning.is_some());
        assert!(warning.unwrap().contains("root.rotation"));
    }

    #[test]
    fn matched_endpoints_do_not_warn() {
        assert!(loop_continuity_warning(&clip(true, 0.0)).is_none());
    }

    #[test]
    fn non_looping_never_warns() {
        assert!(loop_continuity_warning(&clip(false, 90.0)).is_none());
    }
}
