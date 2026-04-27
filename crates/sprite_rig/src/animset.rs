use bevy::{
    animation::{AnimationEvent, AnimationEventTrigger},
    prelude::*,
};
use lightyear::prelude::PredictionDisable;
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

/// Distinguishes the source of a layer entry, used by `cleanup_finished_ability_layers`
/// to decide whether to retain or drop it.
#[derive(Debug, Clone)]
pub enum AnimLayerSource {
    /// The permanent locomotion blend; never dropped.
    Locomotion,
    /// The override-mode side of a transient ability animation, tied to a specific
    /// `ActiveAbility` entity.
    AbilityOverride { ability_entity: Entity },
    /// The additive-mode side of a transient ability animation, tied to a specific
    /// `ActiveAbility` entity.
    AbilityAdditive { ability_entity: Entity },
}

/// Per-bone composition rule for a layer's contribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimLayerMode {
    /// Layer's claimed bones are written by this layer alone; lower-priority layers are
    /// masked off those bones. Participates in the priority chain.
    Override,
    /// Layer adds delta contributions on top of whatever lower-priority layers wrote;
    /// does NOT displace lower layers. Mutually commutative with other additive layers
    /// — multiple additives summing the same bone all register simultaneously.
    Additive,
}

/// One layer in the per-character animation stack: a node in the `AnimationGraph`
/// plus its bone claims, priority, and composition mode.
///
/// On each layer-set change, every layer's effective node mask is recomputed via
/// `recompute_layer_masks`. Override layers participate in a priority chain that masks
/// lower-priority override claimers off shared bones; additive layers always have mask
/// `!claims` (writing only their claimed bones, never displacing anyone).
#[derive(Debug, Clone)]
pub struct AnimLayer {
    pub id: String,
    pub node_index: AnimationNodeIndex,
    pub claims: AnimationMask,
    /// Priority is meaningful only for `Override` entries. For `Additive` entries it's
    /// stored but unused by mask computation; conventionally `0`.
    pub priority: u32,
    pub mode: AnimLayerMode,
    pub source: AnimLayerSource,
}

/// Per-character stack of currently active animation layers.
///
/// The locomotion entry is the permanent first override entry (priority 0). Ability casts
/// push 0-2 entries (an override layer if the clip has Override-mode bones, an additive
/// layer if it has Additive-mode bones). Override entries are kept sorted ascending by
/// priority so later-cast abilities win on shared bones; additive entries' position in
/// the vec doesn't affect semantics.
#[derive(Component, Default, Debug)]
pub struct ActiveAnimLayers {
    pub entries: Vec<AnimLayer>,
}

impl ActiveAnimLayers {
    /// Picks a priority strictly greater than every existing override entry, so a new
    /// override layer wins on any bone it shares with already-active overrides.
    fn next_override_priority(&self) -> u32 {
        self.entries
            .iter()
            .filter(|e| matches!(e.mode, AnimLayerMode::Override))
            .map(|e| e.priority)
            .max()
            .unwrap_or(0)
            + 1
    }

    /// Sets the runtime weight of an additive-mode layer's clip node. No-op if the layer
    /// isn't found, isn't additive, or isn't currently playing on the given player.
    ///
    /// Weight is multiplicative on the layer's delta contribution: `0.0` disables the
    /// overlay, `1.0` is full effect, intermediate values fade. Override-mode layers are
    /// not affected — they always run at weight 1.0 because mask priority is the
    /// composition rule.
    pub fn set_additive_weight(&self, player: &mut AnimationPlayer, layer_id: &str, weight: f32) {
        let Some(layer) = self
            .entries
            .iter()
            .find(|l| l.id == layer_id && matches!(l.mode, AnimLayerMode::Additive))
        else {
            trace!(layer_id, "set_additive_weight: no matching additive layer");
            return;
        };
        let Some(anim) = player.animation_mut(layer.node_index) else {
            trace!(layer_id, "set_additive_weight: clip not playing on player");
            return;
        };
        anim.set_weight(weight);
    }
}

/// Recomputes each layer's effective node mask. Override-mode entries form a priority
/// chain (highest wins on shared bones); additive-mode entries get a static mask
/// `(!claims) & all_groups` so they only write their claimed bones and never participate
/// in the priority chain.
///
/// Pure function of `entries`. Override entries are processed in descending priority order;
/// additive entries are processed independently.
pub(crate) fn recompute_layer_masks(entries: &[AnimLayer], graph: &mut AnimationGraph) {
    // Override priority chain: walk highest priority first, accumulating claimed bones.
    let mut owned_above: AnimationMask = 0;
    let mut override_entries: Vec<&AnimLayer> = entries
        .iter()
        .filter(|e| matches!(e.mode, AnimLayerMode::Override))
        .collect();
    override_entries.sort_by_key(|e| e.priority);
    for layer in override_entries.iter().rev() {
        let effective = (!layer.claims) | owned_above;
        if let Some(node) = graph.get_mut(layer.node_index) {
            node.mask = effective;
        }
        owned_above |= layer.claims;
    }

    // Additive entries: each gets a static `!claims` mask. They never displace anyone.
    for layer in entries
        .iter()
        .filter(|e| matches!(e.mode, AnimLayerMode::Additive))
    {
        if let Some(node) = graph.get_mut(layer.node_index) {
            node.mask = !layer.claims;
        }
    }
}

/// Pushes 0-2 layer entries per newly-added `ActiveAbility` (one per non-empty side of the
/// ability's `AbilityNodePair`) and recomputes layer masks.
///
/// Override-side: pushed only if the ability's source clip has any `Override`-mode bones.
/// Gets a fresh priority above all existing override entries so it wins on shared bones.
///
/// Additive-side: pushed only if the ability's source clip has any `Additive`-mode bones.
/// Doesn't displace anyone — its delta contributions sum on top of whatever the override
/// system writes for the same bones (or alongside, on bones only it claims).
///
/// Both sides share the same `ability_entity` so cleanup can drop them as a unit.
pub fn trigger_ability_animations(
    added_abilities: Query<(Entity, &ActiveAbility), Added<ActiveAbility>>,
    mut characters: Query<
        (
            &mut AnimationPlayer,
            &AnimSetRef,
            &AnimationGraphHandle,
            &mut ActiveAnimLayers,
        ),
        With<CharacterMarker>,
    >,
    built_graphs: Res<BuiltAnimGraphs>,
    mut graph_assets: ResMut<Assets<AnimationGraph>>,
) {
    for (ability_entity, ability) in &added_abilities {
        let Ok((mut player, animset_ref, graph_handle, mut layers)) =
            characters.get_mut(ability.caster)
        else {
            trace!(caster = ?ability.caster, "ability triggered before animation components attached");
            continue;
        };

        let animset_id = animset_ref.0.id();
        let Some(built_graph) = built_graphs.0.get(&animset_id) else {
            trace!(?animset_id, "ability triggered before animset graph built");
            continue;
        };

        let ability_key = &ability.def_id.0;
        let Some(&pair) = built_graph.ability_nodes.get(ability_key) else {
            warn!(
                ability_id = %ability_key,
                "no animation mapping for ability_id in animset",
            );
            continue;
        };

        if pair.override_claims == 0 && pair.additive_claims == 0 {
            // Source clip declared no animated bones; nothing to play.
            continue;
        }

        // Per-side handling: rebind an existing layer if one already targets this node
        // (rollback re-fired `Added` for the same logical cast — keep the in-flight clip
        // running, just point the layer at the new ability_entity), otherwise push fresh
        // and start the clip.
        if pair.override_claims != 0 {
            if let Some(existing) = layers
                .entries
                .iter_mut()
                .find(|e| e.node_index == pair.override_node)
            {
                existing.source = AnimLayerSource::AbilityOverride { ability_entity };
            } else {
                let priority = layers.next_override_priority();
                layers.entries.push(AnimLayer {
                    id: ability_key.clone(),
                    node_index: pair.override_node,
                    claims: pair.override_claims,
                    priority,
                    mode: AnimLayerMode::Override,
                    source: AnimLayerSource::AbilityOverride { ability_entity },
                });
                let anim = player.play(pair.override_node);
                anim.set_weight(1.0);
            }
        }

        if pair.additive_claims != 0 {
            if let Some(existing) = layers
                .entries
                .iter_mut()
                .find(|e| e.node_index == pair.additive_node)
            {
                existing.source = AnimLayerSource::AbilityAdditive { ability_entity };
            } else {
                layers.entries.push(AnimLayer {
                    id: ability_key.clone(),
                    node_index: pair.additive_node,
                    claims: pair.additive_claims,
                    priority: 0,
                    mode: AnimLayerMode::Additive,
                    source: AnimLayerSource::AbilityAdditive { ability_entity },
                });
                let anim = player.play(pair.additive_node);
                anim.set_weight(1.0);
            }
        }

        if let Some(graph) = graph_assets.get_mut(&graph_handle.0) {
            recompute_layer_masks(&layers.entries, graph);
        }
    }
}

/// Drops layer entries whose ability clip has finished or whose `ActiveAbility` entity has
/// been despawned, recomputes masks, and restarts any locomotion clips that stopped.
pub fn cleanup_finished_ability_layers(
    // `Without<PredictionDisable>` filter: lightyear's `prediction_despawn()` marks the
    // entity disabled-but-not-yet-removed for the rollback window. Without this filter,
    // a tombstoned ability looks alive to `abilities.get(...)` and the layer would never
    // drop until the server's confirmed despawn arrives — much later than the clip ends.
    // (The clip-finished branch still fires correctly; this matters mostly for rollback
    // edge cases and any future ability whose clip outlasts its lifetime.)
    abilities: Query<(), (With<ActiveAbility>, Without<PredictionDisable>)>,
    mut characters: Query<
        (
            &mut AnimationPlayer,
            &AnimSetRef,
            &AnimationGraphHandle,
            &mut ActiveAnimLayers,
        ),
        With<CharacterMarker>,
    >,
    built_graphs: Res<BuiltAnimGraphs>,
    mut graph_assets: ResMut<Assets<AnimationGraph>>,
) {
    for (mut player, animset_ref, graph_handle, mut layers) in &mut characters {
        let Some(built_graph) = built_graphs.0.get(&animset_ref.0.id()) else {
            trace!("character's animset graph not built; skipping layer cleanup");
            continue;
        };

        let mut finished_nodes: Vec<AnimationNodeIndex> = Vec::new();
        layers.entries.retain(|layer| {
            let ability_entity = match &layer.source {
                AnimLayerSource::Locomotion => return true,
                AnimLayerSource::AbilityOverride { ability_entity }
                | AnimLayerSource::AbilityAdditive { ability_entity } => *ability_entity,
            };
            let clip_finished = player
                .animation(layer.node_index)
                .is_none_or(|anim| anim.is_finished());
            let ability_removed = abilities.get(ability_entity).is_err();
            if clip_finished || ability_removed {
                finished_nodes.push(layer.node_index);
                false
            } else {
                true
            }
        });

        if finished_nodes.is_empty() {
            continue;
        }

        for node_idx in &finished_nodes {
            player.stop(*node_idx);
        }

        if let Some(graph) = graph_assets.get_mut(&graph_handle.0) {
            recompute_layer_masks(&layers.entries, graph);
        }

        restart_stopped_locomotion_clips(&mut player, &built_graph.locomotion_entries);
    }
}

/// Re-starts any locomotion clips that stopped playing during ability playback.
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
/// Uses `add_event_fn` to gate on blend weight, suppressing events from near-zero weight
/// clips (e.g. run clip playing at weight 0 while idle).
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

#[cfg(test)]
mod tests {
    use super::*;

    fn override_layer(id: &str, claims: AnimationMask, priority: u32) -> AnimLayer {
        AnimLayer {
            id: id.to_string(),
            node_index: AnimationNodeIndex::new(priority as usize + 1),
            claims,
            priority,
            mode: AnimLayerMode::Override,
            source: if id == "locomotion" {
                AnimLayerSource::Locomotion
            } else {
                AnimLayerSource::AbilityOverride {
                    ability_entity: Entity::from_raw_u32(priority + 100).unwrap(),
                }
            },
        }
    }

    fn additive_layer(id: &str, claims: AnimationMask, node_seed: u32) -> AnimLayer {
        AnimLayer {
            id: id.to_string(),
            node_index: AnimationNodeIndex::new(node_seed as usize + 200),
            claims,
            priority: 0,
            mode: AnimLayerMode::Additive,
            source: AnimLayerSource::AbilityAdditive {
                ability_entity: Entity::from_raw_u32(node_seed + 200).unwrap(),
            },
        }
    }

    /// Computes the effective masks `recompute_layer_masks` would write, without touching a
    /// real `AnimationGraph`. Mirrors the production logic: override layers form a priority
    /// chain (highest wins on shared bones); additive layers get a static `!claims` mask.
    fn effective_masks(entries: &[AnimLayer]) -> Vec<AnimationMask> {
        let mut out = vec![0; entries.len()];

        let mut override_indices: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e.mode, AnimLayerMode::Override))
            .map(|(i, _)| i)
            .collect();
        override_indices.sort_by_key(|i| entries[*i].priority);

        let mut owned_above: AnimationMask = 0;
        for &i in override_indices.iter().rev() {
            let layer = &entries[i];
            out[i] = (!layer.claims) | owned_above;
            owned_above |= layer.claims;
        }

        for (i, layer) in entries.iter().enumerate() {
            if matches!(layer.mode, AnimLayerMode::Additive) {
                out[i] = !layer.claims;
            }
        }

        out
    }

    #[test]
    fn locomotion_alone_excludes_no_real_bone() {
        let all = 0b0111_1111u64; // 7 bones
        let layers = vec![override_layer("locomotion", all, 0)];
        let masks = effective_masks(&layers);
        // Locomotion's effective mask only excludes phantom (non-existent) bone groups
        // outside the 7-bone range; within the range, no bone is excluded.
        assert_eq!(masks[0] & all, 0);
    }

    #[test]
    fn ability_above_locomotion_takes_its_claimed_bones() {
        let all = 0b0111_1111u64;
        let punch_claims = 0b0001_0010u64; // torso + arm_r
        let layers = vec![
            override_layer("locomotion", all, 0),
            override_layer("punch", punch_claims, 1),
        ];
        let masks = effective_masks(&layers);
        // Locomotion is now masked from punch's claimed bones.
        assert_eq!(masks[0] & all, punch_claims);
        // Punch is unmasked from its own claimed bones (within the rig).
        assert_eq!(masks[1] & punch_claims, 0);
    }

    #[test]
    fn higher_priority_ability_wins_on_overlapping_bones() {
        let all = 0b0111_1111u64;
        let gp_claims = 0b0111_1011u64; // all but head
        let punch_claims = 0b0001_0010u64; // torso + arm_r
        let layers = vec![
            override_layer("locomotion", all, 0),
            override_layer("ground_pound", gp_claims, 1),
            override_layer("punch", punch_claims, 2),
        ];
        let masks = effective_masks(&layers);
        // Locomotion is excluded from every bone any higher layer claims.
        assert_eq!(masks[0] & all, gp_claims | punch_claims);
        // Ground pound is excluded from bones punch claims (overlap on torso + arm_r).
        assert_eq!(masks[1] & gp_claims, punch_claims & gp_claims);
        // Punch is unmasked on its claimed bones.
        assert_eq!(masks[2] & punch_claims, 0);
    }

    #[test]
    fn dropping_higher_priority_layer_returns_its_bones_to_lower_layers() {
        let all = 0b0111_1111u64;
        let gp_claims = 0b0111_1011u64;
        let punch_claims = 0b0001_0010u64;
        let with_both = vec![
            override_layer("locomotion", all, 0),
            override_layer("ground_pound", gp_claims, 1),
            override_layer("punch", punch_claims, 2),
        ];
        let after_punch_drop = vec![
            override_layer("locomotion", all, 0),
            override_layer("ground_pound", gp_claims, 1),
        ];

        let with_both_masks = effective_masks(&with_both);
        let after_masks = effective_masks(&after_punch_drop);

        // Before: ground pound was masked from torso + arm_r (punch owned them).
        // After: ground pound regains those bones — its effective mask within the rig is 0.
        assert!(with_both_masks[1] & gp_claims != 0);
        assert_eq!(after_masks[1] & gp_claims, 0);
        // Locomotion's exclusion shrinks to just ground pound's claims.
        assert_eq!(after_masks[0] & all, gp_claims);
    }

    #[test]
    fn additive_layer_does_not_displace_lower_priority_overrides() {
        let all = 0b0111_1111u64;
        let aim_torso = 0b0000_0010u64; // additive overlay on torso only
        let layers = vec![
            override_layer("locomotion", all, 0),
            additive_layer("aim_up", aim_torso, 1),
        ];
        let masks = effective_masks(&layers);
        // Locomotion still owns every bone (additive does not subtract from override claims).
        assert_eq!(masks[0] & all, 0);
        // Additive layer is masked off every bone except torso.
        assert_eq!(masks[1] & aim_torso, 0);
        assert_eq!(masks[1] & all & !aim_torso, all & !aim_torso);
    }

    #[test]
    fn additive_layer_coexists_with_override_on_same_bone() {
        // An override layer claiming arm_r and an additive layer claiming arm_r both run.
        // The override's mask isn't widened by the additive's claim — additive doesn't
        // displace anyone — so both contribute (the override dictates the base pose; the
        // additive layers a delta on top).
        let all = 0b0111_1111u64;
        let punch_claims = 0b0001_0010u64; // torso + arm_r
        let recoil_claims = 0b0001_0000u64; // arm_r additive
        let layers = vec![
            override_layer("locomotion", all, 0),
            override_layer("punch", punch_claims, 1),
            additive_layer("recoil", recoil_claims, 1),
        ];
        let masks = effective_masks(&layers);
        // Locomotion is masked off punch's bones (priority chain), unchanged by additive.
        assert_eq!(masks[0] & all, punch_claims);
        // Punch's effective mask within the rig is 0 over its own claims (it owns them).
        assert_eq!(masks[1] & punch_claims, 0);
        // Recoil writes only arm_r and nothing else.
        assert_eq!(masks[2] & recoil_claims, 0);
        assert_eq!(masks[2] & all & !recoil_claims, all & !recoil_claims);
    }

    #[test]
    fn multiple_additive_layers_are_independent() {
        // Two additive layers both targeting the torso. Neither's mask depends on the
        // other, neither displaces the override base. Both write torso simultaneously
        // (Bevy's Add node sums their delta contributions).
        let all = 0b0111_1111u64;
        let breathe = 0b0000_0010u64; // torso
        let aim = 0b0000_0010u64; // torso
        let layers = vec![
            override_layer("locomotion", all, 0),
            additive_layer("breathe", breathe, 1),
            additive_layer("aim", aim, 2),
        ];
        let masks = effective_masks(&layers);
        // Locomotion still owns every bone.
        assert_eq!(masks[0] & all, 0);
        // Each additive layer writes only its own claim.
        assert_eq!(masks[1] & breathe, 0);
        assert_eq!(masks[2] & aim, 0);
    }
}
