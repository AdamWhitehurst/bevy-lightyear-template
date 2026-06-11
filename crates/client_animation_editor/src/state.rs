use bevy::prelude::*;
use bevy_egui::egui;
use sprite_rig::asset::{SpriteAnimAsset, SpriteAnimSetAsset};
use sprite_rig::{AnimBoneDefaults, AnimSetRef, BuiltAnimGraphs, LoadedAnimHandles};

/// Identifies which animset slot the working clip occupies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipSlot {
    /// Index into `locomotion.entries`.
    Locomotion(usize),
    /// Ability id key in `ability_animations`.
    Ability(String),
    HitReact,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Playback {
    Playing,
    Paused,
}

/// A keyframe channel of a bone timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Rotation,
    Translation,
    Scale,
}

/// What the user has selected in the time views.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Selection {
    #[default]
    None,
    /// A keyframe: bone name, channel, index into that channel's key vec.
    Key {
        bone: String,
        channel: Channel,
        idx: usize,
    },
    /// An `AnimEventKeyframe` index in `working.events`.
    Event(usize),
}

/// The single in-memory working model every editor view reads and every edit mutates.
#[derive(Resource)]
pub struct EditorState {
    /// Working copy of the selected clip's source asset.
    pub working: SpriteAnimAsset,
    /// Working copy of the animset (slot assignments, Phase 12 saves it).
    pub working_set: SpriteAnimSetAsset,
    pub selected_clip: ClipSlot,
    /// Seconds, in `[0, working.duration]`.
    pub playhead: f32,
    pub playback: Playback,
    pub selection: Selection,
    /// Rig bone names in bone-index order; fixes the dope sheet's row order (the working
    /// clip's `bone_timelines` is a `HashMap`).
    pub bone_order: Vec<String>,
}

impl EditorState {
    /// Maps a clip time `t` (seconds) to an x within `track` as a fraction of track width.
    /// Shared by every time view so one playhead lands at the same x everywhere.
    pub fn t_to_x(&self, t: f32, track: egui::Rect) -> f32 {
        let frac = if self.working.duration > 0.0 {
            t / self.working.duration
        } else {
            0.0
        };
        track.left() + frac * track.width()
    }

    /// Inverse of `t_to_x`, clamped to `[0, duration]`.
    pub fn x_to_t(&self, x: f32, track: egui::Rect) -> f32 {
        let frac = ((x - track.left()) / track.width()).clamp(0.0, 1.0);
        frac * self.working.duration
    }

    /// The clip path the selected slot refers to in the working animset, if assigned.
    pub fn selected_clip_path(&self) -> Option<&str> {
        match &self.selected_clip {
            ClipSlot::Locomotion(i) => self
                .working_set
                .locomotion
                .entries
                .get(*i)
                .map(|e| e.clip.as_str()),
            ClipSlot::Ability(id) => self
                .working_set
                .ability_animations
                .get(id)
                .map(|s| s.as_str()),
            ClipSlot::HitReact => self.working_set.hit_react.as_deref(),
        }
    }
}

/// Inserts `EditorState` once the rig's animset and the default clip (first locomotion
/// entry, idle) are loaded, cloning both into the working model. Runs until it succeeds;
/// the `resource_exists` run condition keeps it off afterwards.
pub fn init_editor_state(
    mut commands: Commands,
    rigs: Query<&AnimSetRef>,
    animset_assets: Res<Assets<SpriteAnimSetAsset>>,
    anim_assets: Res<Assets<SpriteAnimAsset>>,
    loaded_handles: Res<LoadedAnimHandles>,
    bone_defaults: Res<AnimBoneDefaults>,
) {
    let Ok(animset_ref) = rigs.single() else {
        trace!("editor rig not spawned yet; EditorState waits");
        return;
    };
    let Some(animset) = animset_assets.get(&animset_ref.0) else {
        trace!("animset not loaded yet; EditorState waits");
        return;
    };
    let default_slot = ClipSlot::Locomotion(0);
    let default_path = &animset.locomotion.entries[0].clip;
    let Some(default_handle) = loaded_handles.0.get(default_path) else {
        trace!("default clip not loaded yet; EditorState waits");
        return;
    };
    let Some(working) = anim_assets.get(default_handle) else {
        trace!("default clip not loaded yet; EditorState waits");
        return;
    };
    let Some(bone_order) = bone_defaults
        .0
        .get(&default_handle.id())
        .map(|defaults| defaults.iter().map(|b| b.name.clone()).collect())
    else {
        trace!("bone defaults not populated yet; EditorState waits");
        return;
    };

    commands.insert_resource(EditorState {
        working: working.clone(),
        working_set: animset.clone(),
        selected_clip: default_slot,
        playhead: 0.0,
        playback: Playback::Playing,
        selection: Selection::None,
        bone_order,
    });
}

/// Re-clones `working` from the newly selected slot's clip asset and resets the playhead.
/// Called by the transport when the user picks a different clip.
pub fn select_clip(
    state: &mut EditorState,
    slot: ClipSlot,
    anim_assets: &Assets<SpriteAnimAsset>,
    loaded_handles: &LoadedAnimHandles,
) {
    state.selected_clip = slot;
    let Some(asset) = state
        .selected_clip_path()
        .and_then(|path| loaded_handles.0.get(path))
        .and_then(|handle| anim_assets.get(handle))
    else {
        // A slot with no assigned/loaded clip is a bug: the transport only offers slots
        // read from the loaded animset.
        panic!(
            "selected clip slot {:?} has no loaded clip asset",
            state.selected_clip
        );
    };
    state.working = asset.clone();
    state.playhead = 0.0;
    state.selection = Selection::None;
}

/// Drives the editor rig's `AnimationPlayer` from the transport: ensures the selected
/// clip's node(s) are playing at speed 0 (the transport owns time for the selected clip —
/// Bevy's natural advance would fight the playhead), advances the playhead by `dt` when
/// `Playing` (wrapping at the clip duration), and seeks the node(s) to the playhead.
///
/// When the selection changes, the previous slot's nodes are released first — abilities
/// and hit_react are stopped (otherwise they stay frozen at their last pose and keep
/// writing their bones over the new selection); a previously selected locomotion node
/// instead gets its natural speed back, since locomotion must keep playing
/// (`update_locomotion_blend_weights` requires it). For ability slots both the override
/// and additive nodes are driven.
pub fn drive_player_from_playhead(
    mut state: ResMut<EditorState>,
    mut players: Query<(&mut AnimationPlayer, &AnimSetRef)>,
    built_graphs: Res<BuiltAnimGraphs>,
    time: Res<Time>,
    mut last_applied: Local<Option<ClipSlot>>,
) {
    let Ok((mut player, animset_ref)) = players.single_mut() else {
        trace!("editor rig player not ready; transport idle");
        return;
    };
    let Some(built) = built_graphs.0.get(&animset_ref.0.id()) else {
        trace!("anim graph not built yet; transport idle");
        return;
    };

    if last_applied.as_ref() != Some(&state.selected_clip) {
        if let Some(previous) = last_applied.as_ref() {
            release_slot_nodes(previous, &state, built, &mut player);
        }
        *last_applied = Some(state.selected_clip.clone());
    }

    if state.playback == Playback::Playing {
        let duration = state.working.duration.max(f32::EPSILON);
        state.playhead = (state.playhead + time.delta_secs()) % duration;
    }

    // Strictly below the clip duration: bevy's advance wraps `seek_time %= duration` even
    // at speed 0, which inverts the (last_seek_time, seek_time) pair when seeked exactly
    // to the duration and panics the event-trigger system on the inverted slice range.
    let applied_t = state
        .playhead
        .min(state.working.duration.next_down())
        .max(0.0);
    for node in slot_nodes(&state.selected_clip, &state.working_set, built) {
        let anim = match player.animation_mut(node) {
            Some(anim) => anim,
            None => player.play(node),
        };
        anim.repeat();
        anim.set_speed(0.0);
        // set_seek_time (not seek_to): no event range is queued, so scrubbing fires no
        // clip events. Transport-driven nodes can't fire them anyway — advance normalizes
        // the seek pair before triggers run; Phase 10 adds editor-side event audio.
        anim.set_seek_time(applied_t);
    }
}

/// Releases the transport's hold on a slot's nodes after the selection moves elsewhere:
/// locomotion nodes return to natural speed (they must keep playing); ability/hit_react
/// nodes are stopped so their frozen pose no longer blends over the new selection.
fn release_slot_nodes(
    slot: &ClipSlot,
    state: &EditorState,
    built: &sprite_rig::animation::BuiltAnimGraph,
    player: &mut AnimationPlayer,
) {
    let nodes = slot_nodes(slot, &state.working_set, built);
    match slot {
        ClipSlot::Locomotion(_) => {
            for node in nodes {
                if let Some(anim) = player.animation_mut(node) {
                    anim.set_speed(1.0);
                }
            }
        }
        ClipSlot::Ability(_) | ClipSlot::HitReact => {
            for node in nodes {
                player.stop(node);
            }
        }
    }
}

/// Resolves the graph node(s) a slot drives. Abilities have an override and an additive
/// node; the others have one.
fn slot_nodes(
    slot: &ClipSlot,
    working_set: &SpriteAnimSetAsset,
    built: &sprite_rig::animation::BuiltAnimGraph,
) -> Vec<AnimationNodeIndex> {
    match slot {
        ClipSlot::Locomotion(i) => built
            .locomotion_entries
            .get(*i)
            .map(|e| vec![e.node_index])
            .unwrap_or_default(),
        ClipSlot::Ability(id) => built
            .ability_nodes
            .get(id)
            .map(|pair| {
                let mut nodes = vec![pair.override_node];
                if pair.additive_claims != 0 {
                    nodes.push(pair.additive_node);
                }
                nodes
            })
            .unwrap_or_default(),
        ClipSlot::HitReact => working_set
            .hit_react
            .as_ref()
            .and_then(|path| built.node_map.get(path))
            .map(|&node| vec![node])
            .unwrap_or_default(),
    }
}
