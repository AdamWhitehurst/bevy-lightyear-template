use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::edit::apply_event_retime;
use crate::panels::paint::{draw_diamond, draw_playhead, track_rect};
use crate::panels::timeline::scrub_to;
use crate::state::{EditorState, Selection};

const LANE_HEIGHT: f32 = 26.0;
const DIAMOND_SIZE: f32 = 6.0;
/// Pixel radius within which a press counts as grabbing an event diamond.
const HIT_TOLERANCE: f32 = 10.0;

/// Bone-independent event lane over the shared `t→x` track: one diamond per
/// `AnimEventKeyframe` in `working.events`. Clicking a diamond selects the event (renamed
/// via the inspector); dragging retimes it live (clamped, re-sorting past neighbors);
/// clicking/dragging empty lane area scrubs like every other lane.
pub fn draw_event_lane(
    mut contexts: EguiContexts,
    mut state: ResMut<EditorState>,
    mut dragging_event: Local<bool>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("egui context not ready; skipping event lane frame");
        return;
    };
    egui::TopBottomPanel::bottom("events")
        .exact_height(LANE_HEIGHT)
        .show(ctx, |ui| {
            draw_lane(ui, &mut state, &mut dragging_event);
        });
}

/// Paints the gutter label, per-event diamonds, and the playhead, then routes pointer
/// interaction with the same press-origin grab logic as the dope sheet.
fn draw_lane(ui: &mut egui::Ui, state: &mut EditorState, dragging_event: &mut bool) {
    let (lane, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ui.available_height()),
        egui::Sense::click_and_drag(),
    );
    let track = track_rect(lane);
    let painter = ui.painter_at(lane);
    let event_color = egui::Color32::from_rgb(120, 180, 220);

    painter.text(
        egui::pos2(lane.left() + 4.0, lane.center().y),
        egui::Align2::LEFT_CENTER,
        "events",
        egui::FontId::proportional(11.0),
        ui.visuals().text_color(),
    );
    painter.line_segment(
        [
            egui::pos2(track.left(), lane.center().y),
            egui::pos2(track.right(), lane.center().y),
        ],
        egui::Stroke::new(1.0, ui.visuals().faint_bg_color),
    );

    let diamonds: Vec<(egui::Pos2, usize)> = state
        .working
        .events
        .iter()
        .enumerate()
        .map(|(idx, event)| {
            let pos = egui::pos2(state.t_to_x(event.time, track), lane.center().y);
            draw_diamond(
                &painter,
                pos,
                DIAMOND_SIZE,
                event_color,
                state.selection == Selection::Event(idx),
            );
            (pos, idx)
        })
        .collect();

    if let Some(pos) = response.interact_pointer_pos() {
        if response.drag_started() {
            // Hit-test the press origin: drag_started fires after the drag threshold,
            // by which time the pointer may have left the diamond.
            let grab_pos = response
                .ctx
                .input(|i| i.pointer.press_origin())
                .unwrap_or(pos);
            match hit_test_event(grab_pos, &diamonds) {
                Some(idx) => {
                    state.selection = Selection::Event(idx);
                    *dragging_event = true;
                }
                None => scrub_to(state, pos.x, track),
            }
        } else if response.clicked() {
            match hit_test_event(pos, &diamonds) {
                Some(idx) => state.selection = Selection::Event(idx),
                None => scrub_to(state, pos.x, track),
            }
        } else if response.dragged() {
            if *dragging_event {
                apply_event_retime(state, state.x_to_t(pos.x, track));
            } else {
                scrub_to(state, pos.x, track);
            }
        }
    }
    if response.drag_stopped() {
        *dragging_event = false;
    }
    draw_playhead(&painter, state.t_to_x(state.playhead, track), lane);
}

/// Returns the index of the event diamond nearest `pos` within `HIT_TOLERANCE`, or `None`.
fn hit_test_event(pos: egui::Pos2, diamonds: &[(egui::Pos2, usize)]) -> Option<usize> {
    diamonds
        .iter()
        .map(|(p, idx)| (p.distance(pos), *idx))
        .filter(|(d, _)| *d <= HIT_TOLERANCE)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, idx)| idx)
}
