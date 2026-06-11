use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use sprite_rig::asset::SpriteAnimAsset;
use sprite_rig::LoadedAnimHandles;

use crate::state::{select_clip, ClipSlot, EditorState, Playback};

/// Clip selector + play/pause + seek slider. Drives `EditorState`; the
/// `drive_player_from_playhead` system applies it to the rig's `AnimationPlayer`.
pub fn draw_transport(
    mut contexts: EguiContexts,
    mut state: ResMut<EditorState>,
    anim_assets: Res<Assets<SpriteAnimAsset>>,
    loaded_handles: Res<LoadedAnimHandles>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("egui context not ready; skipping transport frame");
        return;
    };

    egui::TopBottomPanel::bottom("transport").show(ctx, |ui| {
        ui.horizontal(|ui| {
            draw_clip_selector(ui, &mut state, &anim_assets, &loaded_handles);
            draw_play_pause(ui, &mut state);
            draw_seek_slider(ui, &mut state);
        });
    });
}

/// ComboBox over every animset slot (locomotion entries, abilities, hit_react).
/// Selecting a different slot re-clones the working clip and resets the playhead.
fn draw_clip_selector(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    anim_assets: &Assets<SpriteAnimAsset>,
    loaded_handles: &LoadedAnimHandles,
) {
    let mut selected = state.selected_clip.clone();
    egui::ComboBox::from_id_salt("clip_selector")
        .selected_text(slot_label(&selected, state))
        .show_ui(ui, |ui| {
            for (i, entry) in state.working_set.locomotion.entries.iter().enumerate() {
                ui.selectable_value(
                    &mut selected,
                    ClipSlot::Locomotion(i),
                    format!("locomotion: {}", entry.clip),
                );
            }
            let mut ability_ids: Vec<&String> =
                state.working_set.ability_animations.keys().collect();
            ability_ids.sort();
            for id in ability_ids {
                ui.selectable_value(
                    &mut selected,
                    ClipSlot::Ability(id.clone()),
                    format!("ability: {id}"),
                );
            }
            if state.working_set.hit_react.is_some() {
                ui.selectable_value(&mut selected, ClipSlot::HitReact, "hit_react");
            }
        });
    if selected != state.selected_clip {
        select_clip(state, selected, anim_assets, loaded_handles);
    }
}

fn draw_play_pause(ui: &mut egui::Ui, state: &mut EditorState) {
    let label = match state.playback {
        Playback::Playing => "Pause",
        Playback::Paused => "Play",
    };
    if ui.button(label).clicked() {
        state.playback = match state.playback {
            Playback::Playing => Playback::Paused,
            Playback::Paused => Playback::Playing,
        };
    }
}

fn draw_seek_slider(ui: &mut egui::Ui, state: &mut EditorState) {
    let duration = state.working.duration;
    let response = ui.add(
        egui::Slider::new(&mut state.playhead, 0.0..=duration)
            .text("t")
            .clamping(egui::SliderClamping::Always),
    );
    // Dragging the slider scrubs: hold playback so the pose tracks the handle.
    if response.dragged() {
        state.playback = Playback::Paused;
    }
}

/// Human-readable label for the selected slot.
fn slot_label(slot: &ClipSlot, state: &EditorState) -> String {
    match slot {
        ClipSlot::Locomotion(i) => state
            .working_set
            .locomotion
            .entries
            .get(*i)
            .map(|e| format!("locomotion: {}", e.clip))
            .unwrap_or_else(|| format!("locomotion: <missing {i}>")),
        ClipSlot::Ability(id) => format!("ability: {id}"),
        ClipSlot::HitReact => "hit_react".to_string(),
    }
}
