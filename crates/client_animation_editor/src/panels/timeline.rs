use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::panels::paint::{draw_diamond, draw_playhead, track_rect};
use crate::state::{Channel, EditorState, Playback, Selection};

const RULER_HEIGHT: f32 = 22.0;
const CHANNEL_ROW_HEIGHT: f32 = 18.0;
const BONE_HEADER_HEIGHT: f32 = 18.0;
const DIAMOND_SIZE: f32 = 5.0;
/// Pixel radius within which a click counts as hitting a diamond.
const HIT_TOLERANCE: f32 = 7.0;

/// Timeline panel: time ruler docked above the dope sheet, both sharing the same gutter
/// and `t→x` mapping so the playhead lands at the identical x in each. The initial panel
/// height fits the working clip's rows exactly, capped at a fraction of the window (the
/// ScrollArea handles overflow beyond the cap).
pub fn draw_timeline(mut contexts: EguiContexts, mut state: ResMut<EditorState>) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("egui context not ready; skipping timeline frame");
        return;
    };
    let bones = collect_bone_channels(&state);
    let content_height: f32 = RULER_HEIGHT
        + bones
            .iter()
            .map(|(_, channels)| BONE_HEADER_HEIGHT + channels.len() as f32 * CHANNEL_ROW_HEIGHT)
            .sum::<f32>()
        + 12.0;
    let height_cap = ctx.content_rect().height() * 0.45;
    egui::TopBottomPanel::bottom("timeline")
        .resizable(true)
        .default_height(content_height.min(height_cap))
        .show(ctx, |ui| {
            draw_ruler(ui, &mut state);
            egui::ScrollArea::vertical().show(ui, |ui| {
                draw_dope_sheet(ui, &mut state, &bones);
            });
        });
}

/// Per-bone non-empty channels with their keyframe times, in `bone_order`. Drives both
/// the dope sheet rows and the panel's content-fitted initial height.
fn collect_bone_channels(state: &EditorState) -> Vec<(String, Vec<(Channel, Vec<f32>)>)> {
    state
        .bone_order
        .iter()
        .filter_map(|bone| {
            let timeline = state.working.bone_timelines.get(bone)?;
            let mut channels = Vec::new();
            if !timeline.rotation.is_empty() {
                channels.push((
                    Channel::Rotation,
                    timeline.rotation.iter().map(|k| k.time).collect(),
                ));
            }
            if !timeline.translation.is_empty() {
                channels.push((
                    Channel::Translation,
                    timeline.translation.iter().map(|k| k.time).collect(),
                ));
            }
            if !timeline.scale.is_empty() {
                channels.push((
                    Channel::Scale,
                    timeline.scale.iter().map(|k| k.time).collect(),
                ));
            }
            (!channels.is_empty()).then(|| (bone.clone(), channels))
        })
        .collect()
}

/// Time ruler: tick marks + the shared playhead. Clicking/dragging scrubs (seeks the
/// playhead via `x_to_t`, pausing playback so the pose tracks the pointer).
fn draw_ruler(ui: &mut egui::Ui, state: &mut EditorState) {
    let (lane, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), RULER_HEIGHT),
        egui::Sense::click_and_drag(),
    );
    let track = track_rect(lane);
    let painter = ui.painter_at(lane);

    let tick_color = ui.visuals().weak_text_color();
    let divisions = 10;
    for i in 0..=divisions {
        let t = state.working.duration * i as f32 / divisions as f32;
        let x = state.t_to_x(t, track);
        let major = i % 2 == 0;
        let tick_top = if major {
            lane.top() + 4.0
        } else {
            lane.top() + 10.0
        };
        painter.line_segment(
            [egui::pos2(x, tick_top), egui::pos2(x, lane.bottom())],
            egui::Stroke::new(1.0, tick_color),
        );
        if major && i < divisions {
            painter.text(
                egui::pos2(x + 3.0, lane.top()),
                egui::Align2::LEFT_TOP,
                format!("{t:.2}"),
                egui::FontId::proportional(10.0),
                tick_color,
            );
        }
    }

    if let Some(pos) = response.interact_pointer_pos() {
        if response.clicked() || response.dragged() {
            scrub_to(state, pos.x, track);
        }
    }
    draw_playhead(&painter, state.t_to_x(state.playhead, track), lane);
}

/// Dope sheet: read-only view of `working.bone_timelines` in `bone_order`. One row group
/// per bone with non-empty channel sub-tracks; a diamond per key at `t_to_x(key.time)`.
/// Clicking a diamond selects it (no scrub); clicking empty lane area scrubs.
fn draw_dope_sheet(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    bones: &[(String, Vec<(Channel, Vec<f32>)>)],
) {
    let total_height: f32 = bones
        .iter()
        .map(|(_, channels)| BONE_HEADER_HEIGHT + channels.len() as f32 * CHANNEL_ROW_HEIGHT)
        .sum();
    let (region, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), total_height.max(1.0)),
        egui::Sense::click_and_drag(),
    );
    let track = track_rect(region);
    let painter = ui.painter_at(region);
    let text_color = ui.visuals().text_color();
    let weak_color = ui.visuals().weak_text_color();
    let key_color = egui::Color32::from_rgb(190, 170, 90);

    let mut diamonds: Vec<(egui::Pos2, Selection)> = Vec::new();
    let mut y = region.top();
    for (bone, channels) in bones {
        painter.text(
            egui::pos2(region.left() + 4.0, y + BONE_HEADER_HEIGHT / 2.0),
            egui::Align2::LEFT_CENTER,
            bone,
            egui::FontId::proportional(11.0),
            text_color,
        );
        y += BONE_HEADER_HEIGHT;
        for (channel, times) in channels {
            let row = egui::Rect::from_min_size(
                egui::pos2(region.left(), y),
                egui::vec2(region.width(), CHANNEL_ROW_HEIGHT),
            );
            painter.text(
                egui::pos2(region.left() + 14.0, row.center().y),
                egui::Align2::LEFT_CENTER,
                channel_label(*channel),
                egui::FontId::proportional(10.0),
                weak_color,
            );
            painter.line_segment(
                [
                    egui::pos2(track.left(), row.center().y),
                    egui::pos2(track.right(), row.center().y),
                ],
                egui::Stroke::new(1.0, ui.visuals().faint_bg_color),
            );
            for (idx, time) in times.iter().enumerate() {
                let pos = egui::pos2(state.t_to_x(*time, track), row.center().y);
                let selection = Selection::Key {
                    bone: bone.clone(),
                    channel: *channel,
                    idx,
                };
                draw_diamond(
                    &painter,
                    pos,
                    DIAMOND_SIZE,
                    key_color,
                    state.selection == selection,
                );
                diamonds.push((pos, selection));
            }
            y += CHANNEL_ROW_HEIGHT;
        }
    }

    if let Some(pos) = response.interact_pointer_pos() {
        if response.clicked() {
            match hit_test_diamond(pos, &diamonds) {
                Some(selection) => state.selection = selection,
                None => scrub_to(state, pos.x, track),
            }
        } else if response.dragged() && hit_test_diamond(pos, &diamonds).is_none() {
            scrub_to(state, pos.x, track);
        }
    }
    draw_playhead(&painter, state.t_to_x(state.playhead, track), region);
}

/// Returns the `Selection` for the diamond nearest `pos` within `HIT_TOLERANCE`, or `None`.
fn hit_test_diamond(pos: egui::Pos2, diamonds: &[(egui::Pos2, Selection)]) -> Option<Selection> {
    diamonds
        .iter()
        .map(|(p, sel)| (p.distance(pos), sel))
        .filter(|(d, _)| *d <= HIT_TOLERANCE)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, sel)| sel.clone())
}

/// Seeks the playhead to the time under `x`, pausing playback so the pose holds.
fn scrub_to(state: &mut EditorState, x: f32, track: egui::Rect) {
    state.playhead = state.x_to_t(x, track);
    state.playback = Playback::Paused;
}

fn channel_label(channel: Channel) -> &'static str {
    match channel {
        Channel::Rotation => "rotation",
        Channel::Translation => "translation",
        Channel::Scale => "scale",
    }
}
