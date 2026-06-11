use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use sprite_rig::asset::{BoneTimeline, CurveType};

use crate::edit::{apply_key_edit, KeyValue};
use crate::eval::{rotation_keys, sample_scalar, sample_vec2, vec2_keys};
use crate::panels::paint::{draw_diamond, draw_playhead, track_rect};
use crate::state::{Channel, EditorState, Selection};

const PLOT_HEIGHT: f32 = 160.0;
const DIAMOND_SIZE: f32 = 5.0;
const COMPONENT_NODE_RADIUS: f32 = 3.0;
const X_COLOR: egui::Color32 = egui::Color32::from_rgb(110, 200, 110);
const Y_COLOR: egui::Color32 = egui::Color32::from_rgb(110, 150, 230);
/// Plot samples per lane; polyline resolution (Step segments render flat because the
/// evaluator holds the left value across the segment).
const PLOT_SAMPLES: usize = 256;

/// Pixel radius within which a drag/click grabs a key glyph.
const GRAB_TOLERANCE: f32 = 12.0;
/// Vertical insets keeping glyphs clear of the pane edges (and the panel's resize
/// handle), so edge-value keys stay grabbable.
const VALUE_INSET_TOP: f32 = 18.0;
const VALUE_INSET_BOTTOM: f32 = 10.0;

/// What a curve-view drag edits.
#[derive(Clone, Copy, Debug)]
enum CurveDragTarget {
    /// Scalar diamond: x edits time, y edits the value.
    Scalar,
    /// Vector top diamond: x edits time only.
    Time,
    /// Vector component node: x edits time, y edits this component (false = x, true = y).
    Component { y_component: bool },
}

/// Active curve-view drag, with the value-axis range frozen at drag start so the y
/// mapping stays stable while the edited value moves the range.
#[derive(Clone, Copy, Debug)]
pub struct CurveDrag {
    target: CurveDragTarget,
    min: f32,
    max: f32,
}

/// Curve editor: plots the selected key's channel. Scalar (rotation) → one polyline;
/// vector (translation/scale) → one polyline per component (green = X, blue = Y,
/// legended). The shared playhead overlays at the same x as ruler/dope sheet. Vector
/// keys stack their component nodes at one t under a single diamond. Dragging a glyph
/// retimes (x) and — for value glyphs — revalues (y) the key live.
pub fn draw_curve_editor(
    mut contexts: EguiContexts,
    mut state: ResMut<EditorState>,
    mut drag: Local<Option<CurveDrag>>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("egui context not ready; skipping curve frame");
        return;
    };
    egui::TopBottomPanel::bottom("curve_editor")
        .resizable(true)
        .default_height(PLOT_HEIGHT)
        .show(ctx, |ui| {
            let Selection::Key { bone, channel, idx } = state.selection.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.weak("select a key in the dope sheet to plot its channel");
                });
                return;
            };
            let Some(timeline) = state.working.bone_timelines.get(&bone).cloned() else {
                debug_assert!(false, "selected bone '{bone}' missing from working clip");
                return;
            };
            draw_channel_plot(ui, &mut state, &bone, &timeline, channel, idx, &mut drag);
        });
}

/// Lays out one plot lane, paints the selected channel, and applies glyph drags.
fn draw_channel_plot(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    bone: &str,
    timeline: &BoneTimeline,
    channel: Channel,
    selected_idx: usize,
    drag: &mut Option<CurveDrag>,
) {
    let (lane, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ui.available_height().max(60.0)),
        egui::Sense::click_and_drag(),
    );
    let track = track_rect(lane);
    let painter = ui.painter_at(lane);
    let label_color = ui.visuals().weak_text_color();

    painter.text(
        egui::pos2(lane.left() + 4.0, lane.top() + 4.0),
        egui::Align2::LEFT_TOP,
        format!("{bone}.{}", channel_name(channel)),
        egui::FontId::proportional(11.0),
        ui.visuals().text_color(),
    );

    let mut hit_targets: Vec<(egui::Pos2, usize, CurveDragTarget)> = Vec::new();
    let (min, max) = match channel {
        Channel::Rotation => {
            let keys = rotation_keys(timeline);
            let (min, max) = scalar_range(keys.iter().map(|k| k.1));
            draw_value_axis(&painter, lane, track, min, max, label_color);
            draw_scalar_polyline(&painter, state, track, &keys, min, max, X_COLOR);
            for (idx, (time, value, _)) in keys.iter().enumerate() {
                let pos = egui::pos2(
                    state.t_to_x(*time, track),
                    value_to_y(*value, min, max, track),
                );
                draw_diamond(&painter, pos, DIAMOND_SIZE, X_COLOR, idx == selected_idx);
                hit_targets.push((pos, idx, CurveDragTarget::Scalar));
            }
            (min, max)
        }
        Channel::Translation | Channel::Scale => {
            let keys = vec2_keys(timeline, channel);
            let (min, max) = scalar_range(keys.iter().flat_map(|k| [k.1.x, k.1.y]));
            draw_value_axis(&painter, lane, track, min, max, label_color);
            draw_legend(&painter, track, label_color);
            let x_keys: Vec<(f32, f32, CurveType)> =
                keys.iter().map(|(t, v, c)| (*t, v.x, *c)).collect();
            let y_keys: Vec<(f32, f32, CurveType)> =
                keys.iter().map(|(t, v, c)| (*t, v.y, *c)).collect();
            draw_scalar_polyline(&painter, state, track, &x_keys, min, max, X_COLOR);
            draw_scalar_polyline(&painter, state, track, &y_keys, min, max, Y_COLOR);
            for (idx, (time, value, _)) in keys.iter().enumerate() {
                let x = state.t_to_x(*time, track);
                let selected = idx == selected_idx;
                // Component nodes at their value heights, stacked under one diamond at
                // the lane top marking the shared key time.
                let x_pos = egui::pos2(x, value_to_y(value.x, min, max, track));
                let y_pos = egui::pos2(x, value_to_y(value.y, min, max, track));
                let time_pos = egui::pos2(x, track.top() + DIAMOND_SIZE + 2.0);
                painter.circle_filled(x_pos, COMPONENT_NODE_RADIUS, X_COLOR);
                painter.circle_filled(y_pos, COMPONENT_NODE_RADIUS, Y_COLOR);
                draw_diamond(
                    &painter,
                    time_pos,
                    DIAMOND_SIZE,
                    egui::Color32::from_rgb(190, 170, 90),
                    selected,
                );
                hit_targets.push((
                    x_pos,
                    idx,
                    CurveDragTarget::Component { y_component: false },
                ));
                hit_targets.push((y_pos, idx, CurveDragTarget::Component { y_component: true }));
                hit_targets.push((time_pos, idx, CurveDragTarget::Time));
            }
            (min, max)
        }
    };

    apply_curve_interaction(
        state,
        &response,
        track,
        &hit_targets,
        bone,
        channel,
        min,
        max,
        drag,
    );

    draw_playhead(&painter, state.t_to_x(state.playhead, track), lane);

    // Sampled value-at-playhead readout, mirroring the live pose.
    let readout = match channel {
        Channel::Rotation => format!(
            "{:.2}°",
            sample_scalar(&rotation_keys(timeline), state.playhead)
        ),
        Channel::Translation | Channel::Scale => {
            let v = sample_vec2(&vec2_keys(timeline, channel), state.playhead);
            format!("({:.2}, {:.2})", v.x, v.y)
        }
    };
    painter.text(
        egui::pos2(track.right() - 4.0, lane.bottom() - 4.0),
        egui::Align2::RIGHT_BOTTOM,
        readout,
        egui::FontId::proportional(10.0),
        label_color,
    );
}

/// Applies curve-view pointer interaction: grab-to-drag on key glyphs (select + frozen
/// value range), click-to-select, live `apply_key_edit` while dragging.
#[expect(
    clippy::too_many_arguments,
    reason = "plain painter-context plumbing; bundling into a struct adds nothing"
)]
fn apply_curve_interaction(
    state: &mut EditorState,
    response: &egui::Response,
    track: egui::Rect,
    hit_targets: &[(egui::Pos2, usize, CurveDragTarget)],
    bone: &str,
    channel: Channel,
    min: f32,
    max: f32,
    drag: &mut Option<CurveDrag>,
) {
    if let Some(pos) = response.interact_pointer_pos() {
        if response.drag_started() {
            // Hit-test where the press began: egui fires drag_started only after the
            // pointer has moved past its drag threshold, so the current position may
            // already be off the glyph.
            let grab_pos = response
                .ctx
                .input(|i| i.pointer.press_origin())
                .unwrap_or(pos);
            if let Some((idx, target)) = hit_test_glyph(grab_pos, hit_targets) {
                state.selection = Selection::Key {
                    bone: bone.to_string(),
                    channel,
                    idx,
                };
                *drag = Some(CurveDrag { target, min, max });
            }
        } else if response.clicked() {
            if let Some((idx, _)) = hit_test_glyph(pos, hit_targets) {
                state.selection = Selection::Key {
                    bone: bone.to_string(),
                    channel,
                    idx,
                };
            }
        } else if response.dragged() {
            if let Some(active) = *drag {
                let new_time = state.x_to_t(pos.x, track);
                let new_value = drag_value(state, &active, pos.y, track);
                apply_key_edit(state, new_time, new_value);
            }
        }
    }
    if response.drag_stopped() {
        *drag = None;
    }
}

/// The value payload for the active drag target at pointer height `y`, or `None` for
/// time-only drags.
fn drag_value(
    state: &EditorState,
    drag: &CurveDrag,
    y: f32,
    track: egui::Rect,
) -> Option<KeyValue> {
    match drag.target {
        CurveDragTarget::Time => None,
        CurveDragTarget::Scalar => Some(KeyValue::Scalar(y_to_value(y, drag.min, drag.max, track))),
        CurveDragTarget::Component { y_component } => {
            let Selection::Key { bone, channel, idx } = &state.selection else {
                debug_assert!(false, "curve drag without key selection");
                return None;
            };
            let timeline = state
                .working
                .bone_timelines
                .get(bone)
                .expect("dragged bone present in working clip");
            let mut value = vec2_keys(timeline, *channel)
                .get(*idx)
                .expect("dragged key index in bounds")
                .1;
            let component = y_to_value(y, drag.min, drag.max, track);
            if y_component {
                value.y = component;
            } else {
                value.x = component;
            }
            Some(KeyValue::Vec2(value))
        }
    }
}

/// Returns the key index + drag target of the glyph nearest `pos` within `GRAB_TOLERANCE`.
fn hit_test_glyph(
    pos: egui::Pos2,
    targets: &[(egui::Pos2, usize, CurveDragTarget)],
) -> Option<(usize, CurveDragTarget)> {
    targets
        .iter()
        .map(|(p, idx, target)| (p.distance(pos), idx, target))
        .filter(|(d, _, _)| *d <= GRAB_TOLERANCE)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, idx, target)| (*idx, *target))
}

/// Inverse of `value_to_y` over the same frozen range. Deliberately unclamped: dragging
/// past the band edge keeps extending the value linearly (the plot's axis range re-derives
/// from the keys each frame, so the view follows).
fn y_to_value(y: f32, min: f32, max: f32, track: egui::Rect) -> f32 {
    let band = value_band(track);
    let frac = (band.bottom() - y) / band.height();
    min + frac * (max - min)
}

/// Plots a scalar key set as a sampled polyline across the track width.
fn draw_scalar_polyline(
    painter: &egui::Painter,
    state: &EditorState,
    track: egui::Rect,
    keys: &[(f32, f32, CurveType)],
    min: f32,
    max: f32,
    color: egui::Color32,
) {
    if keys.is_empty() {
        return; // channel without keys plots nothing — expected for sparse timelines
    }
    let points: Vec<egui::Pos2> = (0..=PLOT_SAMPLES)
        .map(|i| {
            let t = state.working.duration * i as f32 / PLOT_SAMPLES as f32;
            egui::pos2(
                state.t_to_x(t, track),
                value_to_y(sample_scalar(keys, t), min, max, track),
            )
        })
        .collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, color)));
}

/// Value range over the plotted keys, padded so flat curves don't sit on the lane edge.
fn scalar_range(values: impl Iterator<Item = f32>) -> (f32, f32) {
    let (mut min, mut max) = (f32::INFINITY, f32::NEG_INFINITY);
    for v in values {
        min = min.min(v);
        max = max.max(v);
    }
    if !min.is_finite() || !max.is_finite() {
        return (0.0, 1.0);
    }
    let pad = ((max - min) * 0.1).max(0.5);
    (min - pad, max + pad)
}

/// The inset vertical band values map into, keeping glyphs clear of pane edges.
fn value_band(track: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(track.left(), track.top() + VALUE_INSET_TOP),
        egui::pos2(track.right(), track.bottom() - VALUE_INSET_BOTTOM),
    )
}

/// Maps a channel value to a lane y (max at top).
fn value_to_y(value: f32, min: f32, max: f32, track: egui::Rect) -> f32 {
    let band = value_band(track);
    let frac = ((value - min) / (max - min)).clamp(0.0, 1.0);
    band.bottom() - frac * band.height()
}

/// Min/max axis labels in the gutter, aligned with the lane's value extremes.
fn draw_value_axis(
    painter: &egui::Painter,
    lane: egui::Rect,
    track: egui::Rect,
    min: f32,
    max: f32,
    color: egui::Color32,
) {
    painter.text(
        egui::pos2(track.left() - 6.0, track.top() + 2.0),
        egui::Align2::RIGHT_TOP,
        format!("{max:.1}"),
        egui::FontId::proportional(10.0),
        color,
    );
    painter.text(
        egui::pos2(track.left() - 6.0, lane.bottom() - 2.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("{min:.1}"),
        egui::FontId::proportional(10.0),
        color,
    );
}

/// X/Y component color legend for vector channels.
fn draw_legend(painter: &egui::Painter, track: egui::Rect, weak: egui::Color32) {
    let font = egui::FontId::proportional(10.0);
    painter.text(
        egui::pos2(track.right() - 24.0, track.top() + 2.0),
        egui::Align2::RIGHT_TOP,
        "x",
        font.clone(),
        X_COLOR,
    );
    painter.text(
        egui::pos2(track.right() - 12.0, track.top() + 2.0),
        egui::Align2::RIGHT_TOP,
        "y",
        font.clone(),
        Y_COLOR,
    );
    painter.text(
        egui::pos2(track.right() - 4.0, track.top() + 2.0),
        egui::Align2::RIGHT_TOP,
        "·",
        font,
        weak,
    );
}

fn channel_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Rotation => "rotation",
        Channel::Translation => "translation",
        Channel::Scale => "scale",
    }
}
