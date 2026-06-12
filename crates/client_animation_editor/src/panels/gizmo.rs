use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{egui, EguiContexts};
use sprite_rig::asset::{CurveType, RotationKeyframe, ScaleKeyframe, TranslationKeyframe};
use sprite_rig::BoneEntities;

use crate::edit::{apply_key_edit, AutoKey, KeyValue};
use crate::eval::{rotation_keys, sample_scalar, sample_vec2, vec2_keys};
use crate::state::{Channel, EditorState, Selection};

/// Which channel a gizmo drag edits: set by the transport's Move/Rotate/Scale toggle,
/// the W/E/R hotkeys, or clicking a dope-sheet channel row (including empty rows — the
/// only way to arm a channel that has no keys yet, which auto-key then bootstraps).
#[derive(Resource)]
pub struct GizmoMode(pub Channel);

impl Default for GizmoMode {
    fn default() -> Self {
        Self(Channel::Translation)
    }
}

/// Pixel radius of a bone gizmo handle and its grab tolerance.
const GIZMO_RADIUS: f32 = 6.0;
const GRAB_TOLERANCE: f32 = 10.0;
/// Keys within this many seconds of the playhead count as "at the playhead".
const KEY_TIME_EPSILON: f32 = 1e-3;
/// Additive world-units→scale-factor sensitivity for scale drags.
const SCALE_PER_WORLD_UNIT: f32 = 0.5;

/// An in-flight bone drag. Everything needed to invert the screen delta is captured at
/// drag start so the mapping stays stable for the whole gesture.
pub struct GizmoDrag {
    start_pointer: egui::Pos2,
    /// The dragged bone's screen position at drag start (rotation pivot).
    bone_screen: Vec2,
    /// Authored value of the edited key at drag start.
    start_value: KeyValue,
    /// Time of the edited key (drags revalue, never retime).
    key_time: f32,
    /// Screen-pixels-per-world-unit basis at the bone's depth: columns are the projected
    /// world X and Y axes. Inverted to turn screen deltas back into world deltas.
    screen_from_world: Mat2,
    channel: Channel,
}

/// Draws a clickable gizmo at each bone's screen position (bone `GlobalTransform`
/// projected through the editor camera, offset into window space by the camera's
/// viewport) and converts screen drags into authored keyframe values:
///   - translation: screen delta → world-XY delta added to the raw authored offset (the
///     default-xy and z_order bakes cancel — drags move the *authored* value);
///   - rotation: screen angle around the bone → degrees (mirror-corrected, since the
///     authored value is degrees baked via `Quat::from_rotation_z` at build time);
///   - scale: world-XY delta → additive xy scale factors (z is forced 1.0 at build).
///
/// The edited channel is the current [`GizmoMode`] (W/E/R hotkeys handled here, gated on
/// `wants_keyboard_input` so typing in a text field never switches modes). The edited
/// key is the one at the playhead; with [`AutoKey`] on, a missing key is created at the
/// playhead from the channel's sampled value. Drags that start over egui UI are ignored
/// (`EguiWantsInput`).
#[allow(clippy::too_many_arguments)]
pub fn draw_bone_gizmos(
    mut contexts: EguiContexts,
    mut state: ResMut<EditorState>,
    mut gizmo_mode: ResMut<GizmoMode>,
    auto_key: Res<AutoKey>,
    egui_wants_input: Res<EguiWantsInput>,
    rigs: Query<&BoneEntities>,
    bone_transforms: Query<&GlobalTransform>,
    parents: Query<&ChildOf>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut drag: Local<Option<GizmoDrag>>,
) {
    let Ok(bone_entities) = rigs.single() else {
        trace!("editor rig not spawned; no gizmos");
        return;
    };
    let Ok((camera, camera_tf)) = cameras.single() else {
        trace!("editor camera missing; no gizmos");
        return;
    };
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("egui context not ready; no gizmos");
        return;
    };

    let viewport_offset = camera
        .viewport
        .as_ref()
        .map(|v| v.physical_position.as_vec2() / ctx.pixels_per_point())
        .unwrap_or(Vec2::ZERO);

    let mut gizmos: Vec<(String, Vec2, Entity)> = Vec::new();
    for (bone, &entity) in &bone_entities.0 {
        let Ok(global) = bone_transforms.get(entity) else {
            trace!(bone, "bone entity without GlobalTransform yet");
            continue;
        };
        let Ok(viewport_pos) = camera.world_to_viewport(camera_tf, global.translation()) else {
            trace!(bone, "bone projects outside the camera frustum");
            continue;
        };
        gizmos.push((bone.clone(), viewport_offset + viewport_pos, entity));
    }

    paint_gizmos(ctx, &state, &gizmos);
    apply_mode_hotkeys(ctx, &mut gizmo_mode);

    let pointer_pos = ctx.input(|i| i.pointer.latest_pos());
    let primary_down = ctx.input(|i| i.pointer.primary_down());

    match (&mut *drag, primary_down) {
        // Maybe start a drag: pointer pressed outside egui UI, on a gizmo.
        (None, true) => {
            if egui_wants_input.wants_any_pointer_input() {
                return; // pointer interacting with egui panels — not a bone drag
            }
            let Some(pos) = pointer_pos else {
                return; // no pointer position this frame — nothing to hit-test
            };
            let Some((bone, bone_screen, bone_entity)) = nearest_gizmo(pos, &gizmos) else {
                return; // press on empty viewport space — not a bone drag
            };
            // The authored-value space is the bone's direct parent's frame.
            let parent_rotation = parents
                .get(bone_entity)
                .ok()
                .and_then(|child_of| bone_transforms.get(child_of.parent()).ok())
                .map(|parent_global| parent_global.rotation())
                .unwrap_or(Quat::IDENTITY);
            *drag = start_drag(
                &mut state,
                gizmo_mode.0,
                auto_key.0,
                bone,
                bone_screen,
                pos,
                camera,
                camera_tf,
                viewport_offset,
                parent_rotation,
            );
        }
        // Continue an active drag.
        (Some(active), true) => {
            if let Some(pos) = pointer_pos {
                let new_value = screen_delta_to_bone_value(active, pos);
                apply_key_edit(&mut state, active.key_time, Some(new_value));
            }
        }
        // Pointer released: end the drag.
        (Some(_), false) => *drag = None,
        (None, false) => {}
    }
}

/// Converts the current pointer position into the dragged key's new authored value,
/// inverting the build-time bakes captured in `drag`.
pub fn screen_delta_to_bone_value(drag: &GizmoDrag, pointer: egui::Pos2) -> KeyValue {
    let delta = Vec2::new(
        pointer.x - drag.start_pointer.x,
        pointer.y - drag.start_pointer.y,
    );
    match (drag.channel, drag.start_value) {
        (Channel::Translation, KeyValue::Vec2(start)) => {
            let world_delta = drag.screen_from_world.inverse() * delta;
            KeyValue::Vec2(start + world_delta)
        }
        (Channel::Scale, KeyValue::Vec2(start)) => {
            let world_delta = drag.screen_from_world.inverse() * delta;
            KeyValue::Vec2(start + world_delta * SCALE_PER_WORLD_UNIT)
        }
        (Channel::Rotation, KeyValue::Scalar(start)) => {
            let angle_of =
                |p: egui::Pos2| (p.y - drag.bone_screen.y).atan2(p.x - drag.bone_screen.x);
            let screen_delta = angle_of(pointer) - angle_of(drag.start_pointer);
            // A mirrored screen basis (negative determinant — the usual y-down case)
            // shows world-CCW as screen-CW; flip so the value matches world rotation.
            let sign = if drag.screen_from_world.determinant() < 0.0 {
                -1.0
            } else {
                1.0
            };
            KeyValue::Scalar(start + sign * screen_delta.to_degrees())
        }
        (channel, value) => {
            panic!("gizmo drag with mismatched channel/value: {channel:?}/{value:?}")
        }
    }
}

/// Switches [`GizmoMode`] on W/E/R (move/rotate/scale, the DCC convention). Gated on
/// `wants_keyboard_input`: while any egui widget (e.g. a text field) has keyboard focus,
/// keystrokes are text, not hotkeys.
fn apply_mode_hotkeys(ctx: &egui::Context, gizmo_mode: &mut GizmoMode) {
    if ctx.wants_keyboard_input() {
        return; // a text field owns the keyboard — expected, not a hotkey context
    }
    ctx.input(|i| {
        if i.key_pressed(egui::Key::W) {
            gizmo_mode.0 = Channel::Translation;
        }
        if i.key_pressed(egui::Key::E) {
            gizmo_mode.0 = Channel::Rotation;
        }
        if i.key_pressed(egui::Key::R) {
            gizmo_mode.0 = Channel::Scale;
        }
    });
}

/// Resolves the key a bone drag edits and captures the drag basis. Returns `None` (with a
/// trace) when no key exists at the playhead and auto-key is off.
#[expect(
    clippy::too_many_arguments,
    reason = "drag-start capture needs the full projection context once"
)]
fn start_drag(
    state: &mut EditorState,
    channel: Channel,
    auto_key: bool,
    bone: String,
    bone_screen: Vec2,
    pointer: egui::Pos2,
    camera: &Camera,
    camera_tf: &GlobalTransform,
    viewport_offset: Vec2,
    parent_rotation: Quat,
) -> Option<GizmoDrag> {
    let Some((key_idx, key_time, start_value)) =
        resolve_target_key(state, &bone, channel, auto_key)
    else {
        trace!(
            bone,
            ?channel,
            "no key at playhead and auto-key off; drag ignored"
        );
        return None;
    };
    state.selection = Selection::Key {
        bone: bone.clone(),
        channel,
        idx: key_idx,
    };

    // Build the screen-from-parent-local basis at the bone's depth: authored translation
    // offsets live in the bone's PARENT space, and the billboard system rotates joint
    // roots 180° to face the camera — so parent-local X/Y, not world X/Y, are what a
    // drag must invert (this is also what keeps rotation's mirror sign correct).
    let bone_world = state_bone_world(camera, camera_tf, bone_screen, viewport_offset)?;
    let project = |world: Vec3| -> Option<Vec2> {
        camera
            .world_to_viewport(camera_tf, world)
            .ok()
            .map(|v| viewport_offset + v)
    };
    let origin = project(bone_world)?;
    let x_axis = project(bone_world + parent_rotation * Vec3::X)? - origin;
    let y_axis = project(bone_world + parent_rotation * Vec3::Y)? - origin;

    Some(GizmoDrag {
        start_pointer: pointer,
        bone_screen,
        start_value,
        key_time,
        screen_from_world: Mat2::from_cols(x_axis, y_axis),
        channel,
    })
}

/// The world position whose projection is `bone_screen` — recovered by un-projecting the
/// gizmo's screen position back through the camera at the rig plane's depth (z = 0).
fn state_bone_world(
    camera: &Camera,
    camera_tf: &GlobalTransform,
    bone_screen: Vec2,
    viewport_offset: Vec2,
) -> Option<Vec3> {
    let ray = camera
        .viewport_to_world(camera_tf, bone_screen - viewport_offset)
        .ok()?;
    // The rig is billboarded around the z = 0 plane the camera looks at.
    let distance = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Z))?;
    Some(ray.get_point(distance))
}

/// Finds (or, with auto-key, creates) the dragged channel's key at the playhead. Returns
/// `(index, time, authored value)`.
fn resolve_target_key(
    state: &mut EditorState,
    bone: &str,
    channel: Channel,
    auto_key: bool,
) -> Option<(usize, f32, KeyValue)> {
    let playhead = state.playhead;
    let timeline = state
        .working
        .bone_timelines
        .entry(bone.to_string())
        .or_default();

    let existing = match channel {
        Channel::Rotation => timeline
            .rotation
            .iter()
            .position(|k| (k.time - playhead).abs() <= KEY_TIME_EPSILON)
            .map(|i| {
                (
                    i,
                    timeline.rotation[i].time,
                    KeyValue::Scalar(timeline.rotation[i].value),
                )
            }),
        Channel::Translation => timeline
            .translation
            .iter()
            .position(|k| (k.time - playhead).abs() <= KEY_TIME_EPSILON)
            .map(|i| {
                (
                    i,
                    timeline.translation[i].time,
                    KeyValue::Vec2(timeline.translation[i].value),
                )
            }),
        Channel::Scale => timeline
            .scale
            .iter()
            .position(|k| (k.time - playhead).abs() <= KEY_TIME_EPSILON)
            .map(|i| {
                (
                    i,
                    timeline.scale[i].time,
                    KeyValue::Vec2(timeline.scale[i].value),
                )
            }),
    };
    if existing.is_some() {
        return existing;
    }
    if !auto_key {
        return None;
    }

    // Auto-key: insert a key at the playhead holding the channel's current sampled value
    // (or the channel identity when the channel has no keys yet).
    let inserted = match channel {
        Channel::Rotation => {
            let keys = rotation_keys(timeline);
            let value = if keys.is_empty() {
                0.0
            } else {
                sample_scalar(&keys, playhead)
            };
            let idx = timeline.rotation.partition_point(|k| k.time <= playhead);
            timeline.rotation.insert(
                idx,
                RotationKeyframe {
                    time: playhead,
                    value,
                    curve: CurveType::Linear,
                },
            );
            (idx, playhead, KeyValue::Scalar(value))
        }
        Channel::Translation => {
            let keys = vec2_keys(timeline, channel);
            let value = if keys.is_empty() {
                Vec2::ZERO
            } else {
                sample_vec2(&keys, playhead)
            };
            let idx = timeline.translation.partition_point(|k| k.time <= playhead);
            timeline.translation.insert(
                idx,
                TranslationKeyframe {
                    time: playhead,
                    value,
                    curve: CurveType::Linear,
                },
            );
            (idx, playhead, KeyValue::Vec2(value))
        }
        Channel::Scale => {
            let keys = vec2_keys(timeline, channel);
            let value = if keys.is_empty() {
                Vec2::ONE
            } else {
                sample_vec2(&keys, playhead)
            };
            let idx = timeline.scale.partition_point(|k| k.time <= playhead);
            timeline.scale.insert(
                idx,
                ScaleKeyframe {
                    time: playhead,
                    value,
                    curve: CurveType::Linear,
                },
            );
            (idx, playhead, KeyValue::Vec2(value))
        }
    };
    state.clip_dirty = true;
    Some(inserted)
}

/// Paints a circle per bone, highlighted when the bone owns the current selection.
fn paint_gizmos(ctx: &egui::Context, state: &EditorState, gizmos: &[(String, Vec2, Entity)]) {
    let painter = ctx.layer_painter(egui::LayerId::background());
    for (bone, screen, _) in gizmos {
        let selected = matches!(
            &state.selection,
            Selection::Key { bone: b, .. } if b == bone
        );
        let (fill, stroke) = if selected {
            (
                egui::Color32::from_rgb(240, 200, 80),
                egui::Stroke::new(2.0, egui::Color32::WHITE),
            )
        } else {
            (
                egui::Color32::from_rgba_unmultiplied(200, 200, 220, 140),
                egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
            )
        };
        painter.circle(egui::pos2(screen.x, screen.y), GIZMO_RADIUS, fill, stroke);
    }
}

/// The gizmo nearest `pos` within `GRAB_TOLERANCE`.
fn nearest_gizmo(
    pos: egui::Pos2,
    gizmos: &[(String, Vec2, Entity)],
) -> Option<(String, Vec2, Entity)> {
    gizmos
        .iter()
        .map(|(bone, screen, entity)| {
            let d = (Vec2::new(pos.x, pos.y) - *screen).length();
            (d, bone, screen, entity)
        })
        .filter(|(d, _, _, _)| *d <= GRAB_TOLERANCE)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, bone, screen, entity)| (bone.clone(), *screen, *entity))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(channel: Channel, start_value: KeyValue, screen_from_world: Mat2) -> GizmoDrag {
        GizmoDrag {
            start_pointer: egui::pos2(0.0, 0.0),
            bone_screen: Vec2::ZERO,
            start_value,
            key_time: 0.25,
            screen_from_world,
            channel,
        }
    }

    /// y-down screen with 2 px per world unit: world X → screen +x, world Y → screen -y.
    fn y_down_2px() -> Mat2 {
        Mat2::from_cols(Vec2::new(2.0, 0.0), Vec2::new(0.0, -2.0))
    }

    #[test]
    fn translation_inverts_screen_basis() {
        let d = drag(
            Channel::Translation,
            KeyValue::Vec2(Vec2::new(1.0, 1.0)),
            y_down_2px(),
        );
        // 4 px right, 6 px up on screen → world (+2, +3).
        let KeyValue::Vec2(v) = screen_delta_to_bone_value(&d, egui::pos2(4.0, -6.0)) else {
            panic!("translation drag must yield a vec2");
        };
        assert_eq!(v, Vec2::new(3.0, 4.0));
    }

    #[test]
    fn rotation_inverts_mirrored_screen_angle_to_degrees() {
        let mut d = drag(Channel::Rotation, KeyValue::Scalar(10.0), y_down_2px());
        d.start_pointer = egui::pos2(10.0, 0.0);
        // Pointer moves to screen-up around the pivot: screen angle -90°; the y-down
        // mirror (det < 0) flips it to +90° world-CCW. Authored degrees: 10 + 90.
        let KeyValue::Scalar(deg) = screen_delta_to_bone_value(&d, egui::pos2(0.0, -10.0)) else {
            panic!("rotation drag must yield a scalar");
        };
        assert!((deg - 100.0).abs() < 1e-3, "got {deg}");
    }

    #[test]
    fn scale_applies_world_delta_with_sensitivity() {
        let d = drag(Channel::Scale, KeyValue::Vec2(Vec2::ONE), y_down_2px());
        let KeyValue::Vec2(v) = screen_delta_to_bone_value(&d, egui::pos2(4.0, 0.0)) else {
            panic!("scale drag must yield a vec2");
        };
        assert_eq!(v, Vec2::new(1.0 + 2.0 * SCALE_PER_WORLD_UNIT, 1.0));
    }
}
