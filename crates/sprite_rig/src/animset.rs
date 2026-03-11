use std::time::Duration;

use bevy::{animation::AnimationEvent, animation::AnimationEventTrigger, prelude::*};
use protocol::{ActiveAbility, CharacterMarker};

use crate::animation::{BuiltAnimGraphs, LocomotionState};
use crate::asset::AnimEventKeyframe;
use crate::spawn::AnimSetRef;

/// Animation event fired at authored keyframe times during clip playback.
#[derive(AnimationEvent, Clone)]
pub struct AnimationEventFired {
    pub event_name: String,
}

/// Minimum blend weight below which animation events are suppressed.
const EVENT_WEIGHT_THRESHOLD: f32 = 0.01;

/// Crossfade duration when transitioning to an ability animation.
const ABILITY_CROSSFADE: Duration = Duration::from_millis(80);

/// Triggers ability animation playback when an `ActiveAbility` is first added.
///
/// Suppresses locomotion blending and crossfades to the ability clip identified
/// by `ActiveAbility::def_id` in the character's built animation graph.
pub fn trigger_ability_animations(
    added_abilities: Query<&ActiveAbility, Added<ActiveAbility>>,
    mut characters: Query<
        (
            &mut AnimationPlayer,
            &mut AnimationTransitions,
            &mut LocomotionState,
            &AnimSetRef,
        ),
        With<CharacterMarker>,
    >,
    built_graphs: Res<BuiltAnimGraphs>,
) {
    for ability in &added_abilities {
        let Ok((mut player, mut transitions, mut loco_state, animset_ref)) =
            characters.get_mut(ability.caster)
        else {
            continue; // caster may not have animation components yet during startup
        };

        let animset_id = animset_ref.0.id();
        let Some(built_graph) = built_graphs.0.get(&animset_id) else {
            continue; // graph not built yet — expected during startup
        };

        let ability_key = &ability.def_id.0;
        let Some(&node_idx) = built_graph.node_map.get(ability_key) else {
            warn!(
                ability_id = %ability_key,
                "no animation mapping for ability_id in animset"
            );
            continue;
        };

        transitions.play(&mut player, node_idx, ABILITY_CROSSFADE);
        loco_state.active = false;
    }
}

/// Restores locomotion blending when a character's ability animation ends.
///
/// Detects the end condition as: `LocomotionState.active == false` and no
/// `ActiveAbility` entity references this character as its caster.
pub fn return_to_locomotion(
    abilities: Query<&ActiveAbility>,
    mut characters: Query<
        (
            &mut LocomotionState,
            &mut AnimationPlayer,
            &AnimSetRef,
            Entity,
        ),
        With<CharacterMarker>,
    >,
    built_graphs: Res<BuiltAnimGraphs>,
) {
    for (mut loco_state, mut player, animset_ref, entity) in &mut characters {
        if loco_state.active {
            continue; // already in locomotion mode
        }

        let still_casting = abilities.iter().any(|a| a.caster == entity);
        if still_casting {
            continue; // ability still active — wait for it to end
        }

        loco_state.active = true;

        let built_graph = built_graphs
            .0
            .get(&animset_ref.0.id())
            .expect("LocomotionState exists but graph not built");

        restart_stopped_locomotion_clips(&mut player, &built_graph.locomotion_entries);
    }
}

/// Re-starts any locomotion clips that stopped playing during the ability animation.
fn restart_stopped_locomotion_clips(
    player: &mut AnimationPlayer,
    entries: &[crate::animation::LocomotionNodeEntry],
) {
    for entry in entries {
        if !player.is_playing_animation(entry.node_index) {
            player.play(entry.node_index).repeat();
        }
    }
}

/// Adds authored animation events to a clip during build.
///
/// Uses `add_event_fn` to gate on blend weight, suppressing events from
/// near-zero weight clips (e.g. run clip playing at weight 0 while idle).
pub fn add_events_to_clip(clip: &mut AnimationClip, events: &[AnimEventKeyframe]) {
    for ev in events {
        let event = AnimationEventFired {
            event_name: ev.name.clone(),
        };
        clip.add_event_fn(ev.time, move |commands, entity, _time, weight| {
            if weight > EVENT_WEIGHT_THRESHOLD {
                commands.trigger_with(event.clone(), AnimationEventTrigger { target: entity });
            }
        });
    }
}

/// Observer that logs animation events as they fire.
pub fn on_animation_event_fired(trigger: On<AnimationEventFired>) {
    let player_entity = trigger.trigger().target;
    let event = trigger.event();
    info!(character = ?player_entity, event = %event.event_name, "animation event fired");
}
