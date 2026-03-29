use bevy::{animation::AnimationEvent, animation::AnimationEventTrigger, prelude::*};
use protocol::{ActiveAbility, CharacterMarker};

use crate::animation::BuiltAnimGraphs;
use crate::asset::AnimEventKeyframe;
use crate::spawn::AnimSetRef;

/// Animation event fired at authored keyframe times during clip playback.
#[derive(AnimationEvent, Clone)]
pub struct AnimationEventFired {
    pub event_name: String,
}

/// Minimum blend weight below which animation events are suppressed.
const EVENT_WEIGHT_THRESHOLD: f32 = 0.01;

/// Tracks the currently active ability animation on a character, so `return_to_locomotion`
/// knows which mask to remove from the locomotion blend node.
#[derive(Component)]
pub struct ActiveAbilityAnim {
    pub ability_id: String,
    pub node_index: AnimationNodeIndex,
}

/// Triggers ability animation playback when an `ActiveAbility` is first added.
///
/// Plays the ability clip directly on the `AnimationPlayer` (not via `AnimationTransitions`)
/// and masks the ability's specified bones out of the locomotion blend node so locomotion
/// only drives bones the ability doesn't touch.
pub fn trigger_ability_animations(
    mut commands: Commands,
    added_abilities: Query<&ActiveAbility, Added<ActiveAbility>>,
    mut characters: Query<
        (&mut AnimationPlayer, &AnimSetRef, &AnimationGraphHandle),
        With<CharacterMarker>,
    >,
    built_graphs: Res<BuiltAnimGraphs>,
    mut graph_assets: ResMut<Assets<AnimationGraph>>,
) {
    for ability in &added_abilities {
        let Ok((mut player, animset_ref, graph_handle)) = characters.get_mut(ability.caster) else {
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

        let Some(&specified_mask) = built_graph.ability_bone_masks.get(ability_key) else {
            warn!(
                ability_id = %ability_key,
                "no bone mask for ability_id in animset"
            );
            continue;
        };

        // Mask the ability's bones out of locomotion so locomotion only drives unaffected bones
        if let Some(graph) = graph_assets.get_mut(&graph_handle.0) {
            if let Some(loco_node) = graph.get_mut(built_graph.locomotion_blend_node) {
                loco_node.add_mask(specified_mask);
            }
        }

        // Play ability clip directly — locomotion clips keep running for unmasked bones
        let anim = player.play(node_idx);
        anim.set_weight(1.0);

        commands.entity(ability.caster).insert(ActiveAbilityAnim {
            ability_id: ability_key.clone(),
            node_index: node_idx,
        });
    }
}

/// Restores full locomotion when a character's ability animation clip finishes or the
/// ability entity is removed — whichever comes first.
pub fn return_to_locomotion(
    mut commands: Commands,
    abilities: Query<&ActiveAbility>,
    mut characters: Query<
        (
            &ActiveAbilityAnim,
            &mut AnimationPlayer,
            &AnimSetRef,
            &AnimationGraphHandle,
            Entity,
        ),
        With<CharacterMarker>,
    >,
    built_graphs: Res<BuiltAnimGraphs>,
    mut graph_assets: ResMut<Assets<AnimationGraph>>,
) {
    for (active_anim, mut player, animset_ref, graph_handle, entity) in &mut characters {
        let clip_finished = player
            .animation(active_anim.node_index)
            .is_none_or(|anim| anim.is_finished());
        let ability_removed = !abilities.iter().any(|a| a.caster == entity);

        if !clip_finished && !ability_removed {
            continue; // animation still playing and ability still active
        }

        let built_graph = built_graphs
            .0
            .get(&animset_ref.0.id())
            .expect("ActiveAbilityAnim exists but graph not built");

        // Remove the bone mask from locomotion blend, restoring full-body locomotion
        if let Some(&specified_mask) = built_graph.ability_bone_masks.get(&active_anim.ability_id) {
            if let Some(graph) = graph_assets.get_mut(&graph_handle.0) {
                if let Some(loco_node) = graph.get_mut(built_graph.locomotion_blend_node) {
                    loco_node.remove_mask(specified_mask);
                }
            }
        }

        // Stop the ability clip
        player.stop(active_anim.node_index);

        // Restart any locomotion clips that stopped during ability
        restart_stopped_locomotion_clips(&mut player, &built_graph.locomotion_entries);

        commands.entity(entity).remove::<ActiveAbilityAnim>();
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
    trace!(character = ?player_entity, event = %event.event_name, "animation event fired");
}
