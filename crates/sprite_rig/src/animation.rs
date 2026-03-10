use std::collections::HashMap;

use avian3d::prelude::LinearVelocity;
use bevy::{
    animation::{animated_field, AnimationTarget, AnimationTargetId},
    math::curve::sample_curves::UnevenSampleAutoCurve,
    prelude::*,
};

use crate::asset::{SpriteAnimAsset, SpriteAnimSetAsset, SpriteRigAsset};
use crate::spawn::{AnimSetRef, BoneEntities};
use crate::RigRegistry;
use protocol::CharacterMarker;

/// Maps `SpriteAnimAsset` ID to the derived `Handle<AnimationClip>`.
#[derive(Resource, Default)]
pub struct BuiltAnimations(pub HashMap<AssetId<SpriteAnimAsset>, Handle<AnimationClip>>);

/// Maps anim asset path string to its strong `Handle<SpriteAnimAsset>`.
///
/// Keeps a strong handle alive so Bevy doesn't garbage-collect the asset before it loads.
#[derive(Resource, Default)]
pub struct LoadedAnimHandles(pub HashMap<String, Handle<SpriteAnimAsset>>);

/// Bone default transform + z-order for animation curve building.
///
/// Combines `BoneDef` position data with slot z-order so animation curves produce
/// correct z values directly, avoiding post-animation z-depth restore races.
#[derive(Clone)]
pub struct BoneAnimDefault {
    pub name: String,
    pub default_xy: Vec2,
    pub z_order: f32,
}

/// Maps each `SpriteAnimAsset` ID to its rig's bone animation defaults.
///
/// Built from animset→rig references so that clip building can convert offset-based
/// translation keyframes to absolute bone positions (with baked z-order) and auto-fill
/// missing bones.
#[derive(Resource, Default)]
pub struct AnimBoneDefaults(pub HashMap<AssetId<SpriteAnimAsset>, Vec<BoneAnimDefault>>);

/// Pre-built animation graph for one animset type, shared across character instances.
pub struct BuiltAnimGraph {
    /// Handle to the graph asset.
    pub graph_handle: Handle<AnimationGraph>,
    /// Maps clip path string to `AnimationNodeIndex`.
    pub node_map: HashMap<String, AnimationNodeIndex>,
    /// Locomotion entries in order of speed_threshold.
    pub locomotion_entries: Vec<LocomotionNodeEntry>,
}

/// A locomotion clip node and its speed threshold for blend weight calculation.
pub struct LocomotionNodeEntry {
    pub node_index: AnimationNodeIndex,
    pub speed_threshold: f32,
}

/// Pre-built animation graphs, one per animset asset.
#[derive(Resource, Default)]
pub struct BuiltAnimGraphs(pub HashMap<AssetId<SpriteAnimSetAsset>, BuiltAnimGraph>);

/// Tracks whether the character is in locomotion mode (vs ability animation).
#[derive(Component)]
pub struct LocomotionState {
    pub active: bool,
}

/// Smoothed blend weights for locomotion clips, lerped toward target each frame.
#[derive(Component)]
pub struct LocomotionBlendWeights {
    pub weights: Vec<f32>,
}

/// Rate at which blend weights converge toward targets (per second).
const BLEND_LERP_SPEED: f32 = 10.0;

/// Populates `AnimBoneDefaults` by resolving each animset's rig and mapping its bone
/// definitions to every animation clip referenced by that animset.
///
/// Runs each frame until all animset clip→rig mappings are established.
pub fn populate_anim_bone_defaults(
    registry: Res<RigRegistry>,
    animset_assets: Res<Assets<SpriteAnimSetAsset>>,
    rig_assets: Res<Assets<SpriteRigAsset>>,
    loaded_handles: Res<LoadedAnimHandles>,
    mut bone_defaults: ResMut<AnimBoneDefaults>,
    asset_server: Res<AssetServer>,
) {
    for entry in registry.entries.values() {
        let Some(animset) = animset_assets.get(&entry.animset_handle) else {
            continue; // not loaded yet — expected during startup
        };
        let rig_handle = asset_server.load::<SpriteRigAsset>(&animset.rig);
        let Some(rig) = rig_assets.get(&rig_handle) else {
            continue; // not loaded yet — expected during startup
        };

        let slot_z_orders: HashMap<&str, f32> = rig
            .slots
            .iter()
            .map(|slot| (slot.bone.as_str(), slot.z_order))
            .collect();

        let bone_anim_defaults: Vec<BoneAnimDefault> = rig
            .bones
            .iter()
            .map(|bone| BoneAnimDefault {
                name: bone.name.clone(),
                default_xy: bone.default_transform.translation,
                z_order: slot_z_orders
                    .get(bone.name.as_str())
                    .copied()
                    .unwrap_or(0.0),
            })
            .collect();

        for clip_path in collect_animset_clip_paths(animset) {
            if let Some(anim_handle) = loaded_handles.0.get(clip_path) {
                let anim_id = anim_handle.id();
                if !bone_defaults.0.contains_key(&anim_id) {
                    bone_defaults.0.insert(anim_id, bone_anim_defaults.clone());
                }
            }
        }
    }
}

/// Loads all animation clips referenced by animset assets and records path-to-id mapping.
///
/// Polls the `RigRegistry` entries each frame until all animset clip paths are loaded.
/// Idempotent: skips animsets whose clips are already in `LoadedAnimHandles`.
pub fn load_animset_clips(
    registry: Res<RigRegistry>,
    animset_assets: Res<Assets<SpriteAnimSetAsset>>,
    asset_server: Res<AssetServer>,
    mut loaded_handles: ResMut<LoadedAnimHandles>,
) {
    for entry in registry.entries.values() {
        let Some(animset) = animset_assets.get(&entry.animset_handle) else {
            continue; // not loaded yet — expected during startup
        };

        let paths: Vec<String> = collect_animset_clip_paths(animset)
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        for clip_path in paths {
            if !loaded_handles.0.contains_key(&clip_path) {
                let handle = asset_server.load::<SpriteAnimAsset>(&clip_path);
                loaded_handles.0.insert(clip_path, handle);
            }
        }
    }
}

/// Collects all clip paths referenced by an animset.
fn collect_animset_clip_paths(animset: &SpriteAnimSetAsset) -> Vec<&str> {
    let mut paths: Vec<&str> = animset
        .locomotion
        .entries
        .iter()
        .map(|e| e.clip.as_str())
        .chain(animset.ability_animations.values().map(|s| s.as_str()))
        .collect();
    if let Some(ref clip_path) = animset.hit_react {
        paths.push(clip_path.as_str());
    }
    paths
}

/// Builds `AnimationClip` assets from loaded `SpriteAnimAsset` data.
///
/// Uses a polling approach: iterates `LoadedAnimHandles` and builds clips for any
/// source assets that are loaded but don't yet have a built clip. Also handles
/// hot-reload via `AssetEvent::Modified`.
pub fn build_animation_clips(
    mut events: MessageReader<AssetEvent<SpriteAnimAsset>>,
    source_assets: Res<Assets<SpriteAnimAsset>>,
    mut clips: ResMut<Assets<AnimationClip>>,
    mut built: ResMut<BuiltAnimations>,
    loaded_handles: Res<LoadedAnimHandles>,
    bone_defaults: Res<AnimBoneDefaults>,
) {
    // Build clips for newly-available source assets (polling handles hot-reload timing)
    for (_path, anim_handle) in loaded_handles.0.iter() {
        let anim_id = anim_handle.id();
        if built.0.contains_key(&anim_id) {
            continue; // already built
        }
        let Some(source) = source_assets.get(anim_id) else {
            continue; // not loaded yet — expected during startup
        };
        let Some(bones) = bone_defaults.0.get(&anim_id) else {
            continue; // rig bone defaults not resolved yet — expected during startup
        };
        let clip = build_clip_from(source, bones);
        let handle = clips.add(clip);
        built.0.insert(anim_id, handle);
    }

    // Handle hot-reload: rebuild in-place when source asset is modified on disk
    for event in events.read() {
        if let AssetEvent::Modified { id } = event {
            let Some(source) = source_assets.get(*id) else {
                continue; // asset removed between event and read — unlikely but harmless
            };
            let Some(handle) = built.0.get(id) else {
                continue; // not yet built — will be caught by polling above next frame
            };
            let Some(bones) = bone_defaults.0.get(id) else {
                continue; // rig bone defaults not resolved yet
            };
            let new_clip = build_clip_from(source, bones);
            let _ = clips.insert(handle.id(), new_clip);
        }
    }
}

/// Converts a `SpriteAnimAsset` into a Bevy `AnimationClip`.
///
/// Uses the rig's bone animation defaults to:
/// 1. Convert offset-based translation keyframes to absolute positions with baked z-order
/// 2. Auto-fill bones not mentioned in the animation with "hold at default" curves,
///    ensuring every clip contributes to every bone for correct blend weighting
fn build_clip_from(anim: &SpriteAnimAsset, bone_defaults: &[BoneAnimDefault]) -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.set_duration(anim.duration);

    for bone_default in bone_defaults {
        let target_id =
            AnimationTargetId::from_names(std::iter::once(&Name::new(bone_default.name.clone())));

        if let Some(timeline) = anim.bone_timelines.get(&bone_default.name) {
            add_rotation_curve(&mut clip, target_id, timeline, anim.duration);
            add_translation_curve(
                &mut clip,
                target_id,
                timeline,
                bone_default.default_xy,
                bone_default.z_order,
                anim.duration,
            );
            add_scale_curve(&mut clip, target_id, timeline);
        } else {
            add_hold_at_default_curves(
                &mut clip,
                target_id,
                bone_default.default_xy,
                bone_default.z_order,
                anim.duration,
            );
        }
    }

    clip
}

/// Adds identity rotation + default-position translation curves for bones not in the animation.
fn add_hold_at_default_curves(
    clip: &mut AnimationClip,
    target_id: AnimationTargetId,
    default_xy: Vec2,
    z_order: f32,
    duration: f32,
) {
    let rot_curve = UnevenSampleAutoCurve::new([(0.0, Quat::IDENTITY), (duration, Quat::IDENTITY)])
        .expect("Hold curve needs 2 keyframes");
    clip.add_curve_to_target(
        target_id,
        AnimatableCurve::new(animated_field!(Transform::rotation), rot_curve),
    );

    let pos = Vec3::new(default_xy.x, default_xy.y, z_order);
    let trans_curve = UnevenSampleAutoCurve::new([(0.0, pos), (duration, pos)])
        .expect("Hold curve needs 2 keyframes");
    clip.add_curve_to_target(
        target_id,
        AnimatableCurve::new(animated_field!(Transform::translation), trans_curve),
    );
}

/// Adds a rotation curve from keyframes, or a hold-at-identity curve if too few keyframes.
fn add_rotation_curve(
    clip: &mut AnimationClip,
    target_id: AnimationTargetId,
    timeline: &crate::asset::BoneTimeline,
    duration: f32,
) {
    if timeline.rotation.len() >= 2 {
        let curve = UnevenSampleAutoCurve::new(
            timeline
                .rotation
                .iter()
                .map(|k| (k.time, Quat::from_rotation_z(k.value.to_radians()))),
        )
        .expect("Rotation timeline needs >= 2 keyframes");
        clip.add_curve_to_target(
            target_id,
            AnimatableCurve::new(animated_field!(Transform::rotation), curve),
        );
    } else {
        let curve = UnevenSampleAutoCurve::new([(0.0, Quat::IDENTITY), (duration, Quat::IDENTITY)])
            .expect("Hold curve needs 2 keyframes");
        clip.add_curve_to_target(
            target_id,
            AnimatableCurve::new(animated_field!(Transform::rotation), curve),
        );
    }
}

/// Adds a translation curve with bone default offset and z-order baked in, or hold-at-default if too few keyframes.
fn add_translation_curve(
    clip: &mut AnimationClip,
    target_id: AnimationTargetId,
    timeline: &crate::asset::BoneTimeline,
    default_xy: Vec2,
    z_order: f32,
    duration: f32,
) {
    if timeline.translation.len() >= 2 {
        let curve = UnevenSampleAutoCurve::new(timeline.translation.iter().map(|k| {
            (
                k.time,
                Vec3::new(default_xy.x + k.value.x, default_xy.y + k.value.y, z_order),
            )
        }))
        .expect("Translation timeline needs >= 2 keyframes");
        clip.add_curve_to_target(
            target_id,
            AnimatableCurve::new(animated_field!(Transform::translation), curve),
        );
    } else {
        let pos = Vec3::new(default_xy.x, default_xy.y, z_order);
        let curve = UnevenSampleAutoCurve::new([(0.0, pos), (duration, pos)])
            .expect("Hold curve needs 2 keyframes");
        clip.add_curve_to_target(
            target_id,
            AnimatableCurve::new(animated_field!(Transform::translation), curve),
        );
    }
}

/// Adds a scale curve from keyframes if enough exist. No auto-fill needed for scale.
fn add_scale_curve(
    clip: &mut AnimationClip,
    target_id: AnimationTargetId,
    timeline: &crate::asset::BoneTimeline,
) {
    if timeline.scale.len() < 2 {
        return; // no scale animation — Bevy's default scale (1,1,1) is correct
    }
    let curve = UnevenSampleAutoCurve::new(
        timeline
            .scale
            .iter()
            .map(|k| (k.time, Vec3::new(k.value.x, k.value.y, 1.0))),
    )
    .expect("Scale timeline needs >= 2 keyframes");
    clip.add_curve_to_target(
        target_id,
        AnimatableCurve::new(animated_field!(Transform::scale), curve),
    );
}

/// Builds per-animset `AnimationGraph` when all referenced clips are ready.
pub fn build_anim_graphs(
    registry: Res<RigRegistry>,
    animset_assets: Res<Assets<SpriteAnimSetAsset>>,
    built_anims: Res<BuiltAnimations>,
    loaded_handles: Res<LoadedAnimHandles>,
    mut built_graphs: ResMut<BuiltAnimGraphs>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    for entry in registry.entries.values() {
        let animset_id = entry.animset_handle.id();
        if built_graphs.0.contains_key(&animset_id) {
            continue; // already built
        }

        let Some(animset) = animset_assets.get(&entry.animset_handle) else {
            continue; // not loaded yet — expected during startup
        };

        if !all_clips_built(animset, &*loaded_handles, &*built_anims) {
            continue; // clips not all built yet — expected during startup
        }

        let (graph, node_map, locomotion_entries) =
            build_graph_for_animset(animset, &*loaded_handles, &*built_anims);
        let graph_handle = graphs.add(graph);

        built_graphs.0.insert(
            animset_id,
            BuiltAnimGraph {
                graph_handle,
                node_map,
                locomotion_entries,
            },
        );
    }
}

/// Returns true when every clip path in the animset has a built `AnimationClip`.
fn all_clips_built(
    animset: &SpriteAnimSetAsset,
    loaded_handles: &LoadedAnimHandles,
    built_anims: &BuiltAnimations,
) -> bool {
    let all_paths = animset
        .locomotion
        .entries
        .iter()
        .map(|e| &e.clip)
        .chain(animset.ability_animations.values())
        .chain(animset.hit_react.iter());

    all_paths.into_iter().all(|path| {
        loaded_handles
            .0
            .get(path)
            .and_then(|handle| built_anims.0.get(&handle.id()))
            .is_some()
    })
}

/// Constructs an `AnimationGraph` with a locomotion blend node and ability clip nodes.
fn build_graph_for_animset(
    animset: &SpriteAnimSetAsset,
    loaded_handles: &LoadedAnimHandles,
    built_anims: &BuiltAnimations,
) -> (
    AnimationGraph,
    HashMap<String, AnimationNodeIndex>,
    Vec<LocomotionNodeEntry>,
) {
    let mut graph = AnimationGraph::new();
    let mut node_map = HashMap::new();
    let mut locomotion_entries = Vec::new();

    let blend_node = graph.add_blend(1.0, graph.root);

    for loco_entry in &animset.locomotion.entries {
        let clip_handle = resolve_clip_handle(&loco_entry.clip, loaded_handles, built_anims);
        let node_idx = graph.add_clip(clip_handle, 1.0, blend_node);
        node_map.insert(loco_entry.clip.clone(), node_idx);
        locomotion_entries.push(LocomotionNodeEntry {
            node_index: node_idx,
            speed_threshold: loco_entry.speed_threshold,
        });
    }

    for (ability_id, clip_path) in &animset.ability_animations {
        let clip_handle = resolve_clip_handle(clip_path, loaded_handles, built_anims);
        let node_idx = graph.add_clip(clip_handle, 1.0, graph.root);
        node_map.insert(ability_id.clone(), node_idx);
    }

    if let Some(ref clip_path) = animset.hit_react {
        let clip_handle = resolve_clip_handle(clip_path, loaded_handles, built_anims);
        let node_idx = graph.add_clip(clip_handle, 1.0, graph.root);
        node_map.insert(clip_path.clone(), node_idx);
    }

    (graph, node_map, locomotion_entries)
}

/// Looks up a built `AnimationClip` handle by its source path.
fn resolve_clip_handle(
    clip_path: &str,
    loaded_handles: &LoadedAnimHandles,
    built_anims: &BuiltAnimations,
) -> Handle<AnimationClip> {
    let anim_handle = loaded_handles
        .0
        .get(clip_path)
        .unwrap_or_else(|| panic!("LoadedAnimHandles missing entry for {clip_path}"));
    built_anims
        .0
        .get(&anim_handle.id())
        .unwrap_or_else(|| panic!("BuiltAnimations missing clip for {clip_path}"))
        .clone()
}

/// Attaches `AnimationPlayer`, `AnimationTarget`, and graph handle to characters with built graphs.
pub fn attach_animation_players(
    mut commands: Commands,
    characters: Query<
        (Entity, &BoneEntities, &AnimSetRef),
        (With<CharacterMarker>, Without<AnimationPlayer>),
    >,
    built_graphs: Res<BuiltAnimGraphs>,
) {
    for (entity, bone_entities, animset_ref) in &characters {
        let animset_id = animset_ref.0.id();
        let Some(built_graph) = built_graphs.0.get(&animset_id) else {
            continue; // graph not built yet — expected during startup
        };

        commands.entity(entity).insert((
            AnimationPlayer::default(),
            AnimationTransitions::default(),
            AnimationGraphHandle(built_graph.graph_handle.clone()),
            LocomotionState { active: true },
        ));

        for (bone_name, &bone_entity) in &bone_entities.0 {
            let target_id =
                AnimationTargetId::from_names(std::iter::once(&Name::new(bone_name.clone())));
            commands.entity(bone_entity).insert(AnimationTarget {
                id: target_id,
                player: entity,
            });
        }
    }
}

/// Starts all locomotion clips on newly-added animation players and initializes blend weights.
pub fn start_locomotion_blend(
    mut commands: Commands,
    mut query: Query<(Entity, &mut AnimationPlayer, &AnimSetRef), Added<AnimationPlayer>>,
    built_graphs: Res<BuiltAnimGraphs>,
) {
    for (entity, mut player, animset_ref) in &mut query {
        let built_graph = built_graphs
            .0
            .get(&animset_ref.0.id())
            .expect("AnimationPlayer attached but graph not built");

        let mut initial_weights = vec![0.0; built_graph.locomotion_entries.len()];
        for (i, entry) in built_graph.locomotion_entries.iter().enumerate() {
            let weight = if i == 0 { 1.0 } else { 0.0 };
            initial_weights[i] = weight;
            let anim = player.play(entry.node_index);
            anim.repeat();
            anim.set_weight(weight);
        }

        commands.entity(entity).insert(LocomotionBlendWeights {
            weights: initial_weights,
        });
    }
}

/// Updates locomotion blend weights based on horizontal velocity, with temporal smoothing.
pub fn update_locomotion_blend_weights(
    mut characters: Query<
        (
            &mut AnimationPlayer,
            &LocomotionState,
            &AnimSetRef,
            &LinearVelocity,
            &mut LocomotionBlendWeights,
        ),
        With<CharacterMarker>,
    >,
    built_graphs: Res<BuiltAnimGraphs>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    let lerp_factor = (BLEND_LERP_SPEED * dt).min(1.0);

    for (mut player, loco_state, animset_ref, velocity, mut blend_weights) in &mut characters {
        if !loco_state.active {
            continue; // locomotion disabled during ability animations
        }
        let Some(built_graph) = built_graphs.0.get(&animset_ref.0.id()) else {
            continue; // graph not built yet — expected during startup
        };
        let speed = velocity.xz().length();
        let target_weights = compute_blend_weights(speed, &built_graph.locomotion_entries);

        debug_assert_eq!(
            blend_weights.weights.len(),
            target_weights.len(),
            "LocomotionBlendWeights length mismatch with locomotion entries"
        );

        for (i, (entry, &target)) in built_graph
            .locomotion_entries
            .iter()
            .zip(target_weights.iter())
            .enumerate()
        {
            let current = &mut blend_weights.weights[i];
            *current += (target - *current) * lerp_factor;

            let anim = player
                .animation_mut(entry.node_index)
                .expect("Locomotion clip must be playing when locomotion is active");
            anim.set_weight(*current);
        }
    }
}

/// Computes 1D linear interpolation blend weights from speed and sorted threshold entries.
///
/// Returns a `Vec<f32>` of weights (summing to 1.0) where at most two adjacent entries are nonzero.
pub fn compute_blend_weights(speed: f32, entries: &[LocomotionNodeEntry]) -> Vec<f32> {
    let mut weights = vec![0.0; entries.len()];
    if entries.is_empty() {
        return weights;
    }
    if entries.len() == 1 {
        weights[0] = 1.0;
        return weights;
    }

    if speed <= entries[0].speed_threshold {
        weights[0] = 1.0;
        return weights;
    }
    if speed >= entries.last().expect("checked len >= 2").speed_threshold {
        *weights.last_mut().expect("checked len >= 2") = 1.0;
        return weights;
    }

    for i in 0..entries.len() - 1 {
        let lo = entries[i].speed_threshold;
        let hi = entries[i + 1].speed_threshold;
        if speed >= lo && speed < hi {
            let t = (speed - lo) / (hi - lo);
            weights[i] = 1.0 - t;
            weights[i + 1] = t;
            return weights;
        }
    }

    debug_assert!(false, "Speed {speed} fell through all threshold ranges");
    weights[0] = 1.0;
    weights
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entries(thresholds: &[f32]) -> Vec<LocomotionNodeEntry> {
        thresholds
            .iter()
            .enumerate()
            .map(|(i, &t)| LocomotionNodeEntry {
                node_index: AnimationNodeIndex::new(i),
                speed_threshold: t,
            })
            .collect()
    }

    #[test]
    fn blend_weights_below_minimum() {
        let entries = make_entries(&[0.0, 2.0, 6.0]);
        let w = compute_blend_weights(0.0, &entries);
        assert_eq!(w, vec![1.0, 0.0, 0.0]);
    }

    #[test]
    fn blend_weights_above_maximum() {
        let entries = make_entries(&[0.0, 2.0, 6.0]);
        let w = compute_blend_weights(10.0, &entries);
        assert_eq!(w, vec![0.0, 0.0, 1.0]);
    }

    #[test]
    fn blend_weights_midpoint() {
        let entries = make_entries(&[0.0, 2.0, 6.0]);
        let w = compute_blend_weights(1.0, &entries);
        assert_eq!(w, vec![0.5, 0.5, 0.0]);
    }

    #[test]
    fn blend_weights_exact_threshold() {
        let entries = make_entries(&[0.0, 2.0, 6.0]);
        let w = compute_blend_weights(2.0, &entries);
        assert_eq!(w, vec![0.0, 1.0, 0.0]);
    }

    #[test]
    fn blend_weights_between_upper_pair() {
        let entries = make_entries(&[0.0, 2.0, 6.0]);
        let w = compute_blend_weights(4.0, &entries);
        assert_eq!(w, vec![0.0, 0.5, 0.5]);
    }

    #[test]
    fn blend_weights_single_entry() {
        let entries = make_entries(&[0.0]);
        let w = compute_blend_weights(5.0, &entries);
        assert_eq!(w, vec![1.0]);
    }

    #[test]
    fn blend_weights_empty() {
        let entries = make_entries(&[]);
        let w = compute_blend_weights(5.0, &entries);
        assert!(w.is_empty());
    }
}
