use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use persistence::{PendingStoreOps, StoreBackend};
use sprite_rig::asset::{SpriteAnimAsset, SpriteAnimSetAsset};
use sprite_rig::{AnimBoneDefaults, AnimSetRef, LoadedAnimHandles};

use crate::state::{assign_new_clip, EditorState, NewClipSlot};
use crate::store::{ClipPath, FsAnimClipStore, FsAnimSetStore};

/// Last save/assign outcome shown in the save bar: queue confirmations and validation
/// errors from the bar itself, completions and failures from `drain_save_results`.
#[derive(Resource, Default)]
pub struct SaveStatus(pub Option<String>);

/// Which slot kind the new-clip form assigns to.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum NewSlotKind {
    #[default]
    Ability,
    Locomotion,
    HitReact,
}

type ClipStore = (
    &'static StoreBackend<ClipPath, SpriteAnimAsset, FsAnimClipStore>,
    &'static mut PendingStoreOps<ClipPath, SpriteAnimAsset>,
);
type SetStore = (
    &'static StoreBackend<String, SpriteAnimSetAsset, FsAnimSetStore>,
    &'static mut PendingStoreOps<String, SpriteAnimSetAsset>,
);

/// Top bar: "Save clip" / "Save animset" buttons, the new-clip assignment form, and the
/// last save status. Saves go through the bevy-persistence ops machinery (spawned on the
/// async pool, drained by `drain_save_results`).
#[allow(clippy::too_many_arguments)]
pub fn draw_save(
    mut contexts: EguiContexts,
    mut state: ResMut<EditorState>,
    mut status: ResMut<SaveStatus>,
    mut clip_store: Query<ClipStore>,
    mut set_store: Query<SetStore>,
    rigs: Query<&AnimSetRef>,
    mut anim_assets: ResMut<Assets<SpriteAnimAsset>>,
    mut animset_assets: ResMut<Assets<SpriteAnimSetAsset>>,
    mut loaded_handles: ResMut<LoadedAnimHandles>,
    mut bone_defaults: ResMut<AnimBoneDefaults>,
    mut new_path: Local<String>,
    mut new_kind: Local<NewSlotKind>,
    mut new_ability_id: Local<String>,
    mut new_threshold: Local<f32>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("egui context not ready; skipping save bar frame");
        return;
    };
    let Ok((clip_backend, mut clip_ops)) = clip_store.single_mut() else {
        trace!("clip store entity not spawned yet; save bar waits");
        return;
    };
    let Ok((set_backend, mut set_ops)) = set_store.single_mut() else {
        trace!("animset store entity not spawned yet; save bar waits");
        return;
    };
    let Ok(animset_ref) = rigs.single() else {
        trace!("editor rig not spawned yet; save bar waits");
        return;
    };

    egui::TopBottomPanel::top("save_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button("Save clip").clicked() {
                save_clip(
                    &state,
                    clip_backend,
                    &mut clip_ops,
                    &mut anim_assets,
                    &loaded_handles,
                    &mut status,
                );
            }
            if ui.button("Save animset").clicked() {
                save_animset(&state, animset_ref, set_backend, &mut set_ops, &mut status);
            }
            ui.separator();

            ui.label("new clip:");
            ui.add(
                egui::TextEdit::singleline(&mut *new_path)
                    .hint_text("anims/humanoid/kick")
                    .desired_width(200.0),
            );
            egui::ComboBox::from_id_salt("new_slot_kind")
                .selected_text(kind_label(*new_kind))
                .show_ui(ui, |ui| {
                    for kind in [
                        NewSlotKind::Ability,
                        NewSlotKind::Locomotion,
                        NewSlotKind::HitReact,
                    ] {
                        ui.selectable_value(&mut *new_kind, kind, kind_label(kind));
                    }
                });
            match *new_kind {
                NewSlotKind::Ability => {
                    ui.add(
                        egui::TextEdit::singleline(&mut *new_ability_id)
                            .hint_text("ability id")
                            .desired_width(90.0),
                    );
                }
                NewSlotKind::Locomotion => {
                    ui.add(
                        egui::DragValue::new(&mut *new_threshold)
                            .speed(0.1)
                            .prefix("threshold: "),
                    );
                }
                NewSlotKind::HitReact => {}
            }
            if ui.button("Assign").clicked() {
                let slot = match *new_kind {
                    NewSlotKind::Ability => NewClipSlot::Ability {
                        id: new_ability_id.trim().to_string(),
                    },
                    NewSlotKind::Locomotion => NewClipSlot::Locomotion {
                        speed_threshold: *new_threshold,
                    },
                    NewSlotKind::HitReact => NewClipSlot::HitReact,
                };
                status.0 = Some(
                    match assign_new_clip(
                        &mut state,
                        &new_path,
                        slot,
                        animset_ref,
                        &mut anim_assets,
                        &mut animset_assets,
                        &mut loaded_handles,
                        &mut bone_defaults,
                    ) {
                        Ok(rel) => format!("assigned {rel} — Save clip + Save animset to persist"),
                        Err(e) => format!("assign failed: {e}"),
                    },
                );
            }

            if let Some(message) = &status.0 {
                ui.separator();
                ui.label(message.as_str());
            }
        });
    });
}

/// Queues a save of the working clip to its slot's path, and syncs the in-memory source
/// asset to the working copy so selecting away and back matches what's on disk (the
/// `Modified` event this emits rebakes the live clip pair, a no-op for current curves).
fn save_clip(
    state: &EditorState,
    backend: &StoreBackend<ClipPath, SpriteAnimAsset, FsAnimClipStore>,
    ops: &mut PendingStoreOps<ClipPath, SpriteAnimAsset>,
    anim_assets: &mut Assets<SpriteAnimAsset>,
    loaded_handles: &LoadedAnimHandles,
    status: &mut SaveStatus,
) {
    let rel = state
        .selected_clip_path()
        .expect("selected slot always has an assigned path")
        .to_string();
    let handle = loaded_handles
        .0
        .get(&rel)
        .unwrap_or_else(|| panic!("no loaded handle for working clip '{rel}'"));
    let _ = anim_assets.insert(handle.id(), state.working.clone());
    ops.spawn_save(
        &backend.0,
        ClipPath {
            rel: rel.clone(),
            bone_order: state.bone_order.clone(),
        },
        state.working.clone(),
    );
    status.0 = Some(format!("saving {rel}…"));
}

/// Queues a save of the working animset to the path its handle was loaded from.
fn save_animset(
    state: &EditorState,
    animset_ref: &AnimSetRef,
    backend: &StoreBackend<String, SpriteAnimSetAsset, FsAnimSetStore>,
    ops: &mut PendingStoreOps<String, SpriteAnimSetAsset>,
    status: &mut SaveStatus,
) {
    let rel = animset_ref
        .0
        .path()
        .expect("editor animset handle is path-loaded")
        .path()
        .to_string_lossy()
        .to_string();
    ops.spawn_save(&backend.0, rel.clone(), state.working_set.clone());
    status.0 = Some(format!("saving {rel}…"));
}

/// Polls both store ops each frame and drains results: completions update the status
/// line; failures are logged via `error!` AND shown in the bar — never silently dropped.
pub fn drain_save_results(
    mut clip_ops: Query<&mut PendingStoreOps<ClipPath, SpriteAnimAsset>>,
    mut set_ops: Query<&mut PendingStoreOps<String, SpriteAnimSetAsset>>,
    mut status: ResMut<SaveStatus>,
) {
    for mut ops in &mut clip_ops {
        ops.poll();
        for done in ops.completed_saves.drain(..) {
            info!(path = %done.key.rel, "clip saved");
            status.0 = Some(format!("saved {}", done.key.rel));
        }
        for failure in ops.save_errors.drain(..) {
            error!(path = %failure.key.rel, error = %failure.error, "clip save FAILED");
            status.0 = Some(format!(
                "SAVE FAILED {}: {}",
                failure.key.rel, failure.error
            ));
        }
    }
    for mut ops in &mut set_ops {
        ops.poll();
        for done in ops.completed_saves.drain(..) {
            info!(path = %done.key, "animset saved");
            status.0 = Some(format!("saved {}", done.key));
        }
        for failure in ops.save_errors.drain(..) {
            error!(path = %failure.key, error = %failure.error, "animset save FAILED");
            status.0 = Some(format!("SAVE FAILED {}: {}", failure.key, failure.error));
        }
    }
}

fn kind_label(kind: NewSlotKind) -> &'static str {
    match kind {
        NewSlotKind::Ability => "ability",
        NewSlotKind::Locomotion => "locomotion",
        NewSlotKind::HitReact => "hit_react",
    }
}
