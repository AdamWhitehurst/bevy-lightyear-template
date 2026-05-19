use bevy::prelude::*;
use protocol::{VoxelBrushEditRequest, VoxelEditRequest};

#[cfg(feature = "spawn-panel")]
use super::gestures::ClientPointerGestureState;
#[cfg(feature = "spawn-panel")]
use super::ownership::{ClientInputOwnershipSnapshot, PointerInputOwner};
#[cfg(feature = "spawn-panel")]
use super::raw::RawClientActions;
#[cfg(feature = "spawn-panel")]
use dev::{panels::spawn::SpawnPanelUi, EditingMode};
#[cfg(feature = "spawn-panel")]
use leafwing_input_manager::prelude::ActionState;
#[cfg(feature = "spawn-panel")]
use lightyear::prelude::Controlled;

/// Client-local terrain command intents produced after ownership gating.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum TerrainCommandIntent {
    BrushStroke(VoxelBrushEditRequest),
    LegacyVoxelEdit(VoxelEditRequest),
}

/// Client-local world-object command intents produced after ownership gating.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum WorldObjectCommandIntent {
    Place,
    Pick,
    Move,
    Rotate { yaw_delta: f32 },
    Delete,
}

/// Emits world-object command intents from owned pointer and keyboard input.
#[cfg(feature = "spawn-panel")]
pub fn write_world_object_command_intents(
    ownership: Res<ClientInputOwnershipSnapshot>,
    gesture: Res<ClientPointerGestureState>,
    editing_mode: Res<EditingMode>,
    action_query: Query<&ActionState<RawClientActions>, With<Controlled>>,
    ui_state: Option<ResMut<SpawnPanelUi>>,
    mut writer: MessageWriter<WorldObjectCommandIntent>,
) {
    let Some(mut ui_state) = ui_state else {
        trace!("write_world_object_command_intents: SpawnPanelUi not ready");
        return;
    };

    write_pointer_world_object_intent(
        &ownership,
        &gesture,
        *editing_mode,
        &action_query,
        &ui_state,
        &mut writer,
    );
    write_panel_world_object_intents(&mut ui_state, &mut writer);
    write_keyboard_world_object_intents(&ownership, *editing_mode, &action_query, &mut writer);
}

#[cfg(feature = "spawn-panel")]
fn write_pointer_world_object_intent(
    ownership: &ClientInputOwnershipSnapshot,
    gesture: &ClientPointerGestureState,
    editing_mode: EditingMode,
    action_query: &Query<&ActionState<RawClientActions>, With<Controlled>>,
    ui_state: &SpawnPanelUi,
    writer: &mut MessageWriter<WorldObjectCommandIntent>,
) {
    let Ok(action_state) = action_query.single() else {
        trace!("write_pointer_world_object_intent: no controlled action state");
        return;
    };
    if !action_state.just_pressed(&RawClientActions::PlaceVoxel) {
        trace!("write_pointer_world_object_intent: primary world-object action not pressed");
        return;
    }

    let pointer_owner = gesture.effective_owner(ownership.pointer);
    if !pointer_owner.allows_world_object() {
        trace!(
            ?pointer_owner,
            "write_pointer_world_object_intent: world-object pointer command suppressed"
        );
        return;
    }

    match editing_mode {
        EditingMode::PlaceDefinition if ui_state.placement.armed => {
            writer.write(WorldObjectCommandIntent::Place);
        }
        EditingMode::PlaceDefinition => {
            trace!("write_pointer_world_object_intent: placement is not armed");
        }
        EditingMode::SelectEdit if ui_state.selection.move_armed => {
            writer.write(WorldObjectCommandIntent::Move);
        }
        EditingMode::SelectEdit if ui_state.selection.cursor_pick_armed => {
            writer.write(WorldObjectCommandIntent::Pick);
        }
        EditingMode::SelectEdit => {
            trace!("write_pointer_world_object_intent: no world-object edit action is armed");
        }
        _ => {
            trace!(
                ?editing_mode,
                "write_pointer_world_object_intent: editing mode does not accept world-object pointer commands"
            );
        }
    }
}

#[cfg(feature = "spawn-panel")]
fn write_panel_world_object_intents(
    ui_state: &mut SpawnPanelUi,
    writer: &mut MessageWriter<WorldObjectCommandIntent>,
) {
    if ui_state.selection.rotate_requested {
        ui_state.selection.rotate_requested = false;
        writer.write(WorldObjectCommandIntent::Rotate {
            yaw_delta: ui_state.selection.rotation_degrees_y,
        });
    }

    if ui_state.selection.delete_requested {
        ui_state.selection.delete_requested = false;
        writer.write(WorldObjectCommandIntent::Delete);
    }
}

#[cfg(feature = "spawn-panel")]
fn write_keyboard_world_object_intents(
    ownership: &ClientInputOwnershipSnapshot,
    editing_mode: EditingMode,
    action_query: &Query<&ActionState<RawClientActions>, With<Controlled>>,
    writer: &mut MessageWriter<WorldObjectCommandIntent>,
) {
    let Ok(action_state) = action_query.single() else {
        trace!("write_keyboard_world_object_intents: no controlled raw action state");
        return;
    };
    if !action_state.just_pressed(&RawClientActions::Delete) {
        trace!("write_keyboard_world_object_intents: delete key not pressed");
        return;
    }
    if !editing_mode.accepts_world_object_commands() {
        trace!(
            ?editing_mode,
            "write_keyboard_world_object_intents: editing mode rejects world-object delete"
        );
        return;
    }
    if !ownership.keyboard.allows_world_object_commands() {
        trace!(
            owner = ?ownership.keyboard,
            "write_keyboard_world_object_intents: keyboard-owned delete suppressed"
        );
        return;
    }

    writer.write(WorldObjectCommandIntent::Delete);
}

#[cfg(feature = "spawn-panel")]
pub fn pointer_owner_allows_world_object_commands(owner: PointerInputOwner) -> bool {
    owner.allows_world_object()
}
