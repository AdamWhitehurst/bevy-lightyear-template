use std::collections::HashMap;

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use lightyear::prelude::*;
use protocol::{CharacterMarker, CharacterType};

use crate::asset::SpriteRigAsset;
use crate::{asset::SpriteAnimSetAsset, RigRegistry};

/// Reference to the rig asset for this character. Triggers rig spawning.
#[derive(Component)]
pub struct SpriteRig(pub Handle<SpriteRigAsset>);

/// Reference to the animset for this character.
#[derive(Component)]
pub struct AnimSetRef(pub Handle<SpriteAnimSetAsset>);

/// Maps bone names to their spawned child entities.
#[derive(Component, Default)]
pub struct BoneEntities(pub HashMap<String, Entity>);

/// Horizontal facing direction derived from velocity.
#[derive(Component, PartialEq, Clone, Copy)]
pub enum Facing {
    Left,
    Right,
}

/// Marker for the billboard child entity that parents all root bone entities.
#[derive(Component)]
pub struct RigBillboard;

/// Preserves a bone's z-depth so animation curves (which overwrite all 3 translation axes) don't
/// flatten the draw order. A post-animation system restores `Transform::translation.z` from this.
#[derive(Component)]
pub struct BoneZOrder(pub f32);

/// Looks up `RigRegistry` when `CharacterType` is added and inserts rig components.
pub fn resolve_character_rig(
    mut commands: Commands,
    query: Query<
        (Entity, &CharacterType),
        (
            Added<CharacterType>,
            Without<SpriteRig>,
            Or<(With<Predicted>, With<Replicated>, With<Interpolated>)>,
            With<CharacterMarker>,
        ),
    >,
    registry: Res<RigRegistry>,
) {
    for (entity, char_type) in &query {
        let entry = registry
            .entries
            .get(char_type)
            .expect("RigRegistry missing entry for CharacterType");
        commands.entity(entity).insert((
            SpriteRig(entry.rig_handle.clone()),
            AnimSetRef(entry.animset_handle.clone()),
            Facing::Right,
        ));
    }
}

/// Spawns bone hierarchy under a `RigBillboard` when `SpriteRig` is added.
pub fn spawn_sprite_rigs(
    mut commands: Commands,
    query: Query<(Entity, &SpriteRig), Added<SpriteRig>>,
    rig_assets: Res<Assets<SpriteRigAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, sprite_rig) in &query {
        let rig = rig_assets
            .get(&sprite_rig.0)
            .expect("SpriteRigAsset must be loaded before gameplay (AppState::Ready)");

        let slot_lookup = build_slot_lookup(rig);
        let sorted_bones = topological_sort_bones(&rig.bones);

        let billboard_id = commands
            .spawn((
                RigBillboard,
                Name::new("RigBillboard"),
                Transform::default(),
            ))
            .id();
        commands.entity(entity).add_child(billboard_id);

        let bone_map = spawn_bone_hierarchy(
            &mut commands,
            billboard_id,
            &sorted_bones,
            &slot_lookup,
            &mut *meshes,
            &mut *materials,
        );

        commands.entity(entity).insert(BoneEntities(bone_map));
    }
}

/// Spawns bone entities in parent-child hierarchy. Only bones with a slot get a visible mesh.
fn spawn_bone_hierarchy(
    commands: &mut Commands,
    billboard_id: Entity,
    sorted_bones: &[&crate::asset::BoneDef],
    slot_lookup: &HashMap<&str, (f32, Vec2)>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> HashMap<String, Entity> {
    let mut bone_map = HashMap::<String, Entity>::new();

    for bone_def in sorted_bones {
        let transform = bone_transform_from_def(bone_def, slot_lookup);

        let z_order = transform.translation.z;
        let mut bone_cmds = commands.spawn((
            Name::new(bone_def.name.clone()),
            transform,
            BoneZOrder(z_order),
        ));

        if let Some(&(_z_order, size)) = slot_lookup.get(bone_def.name.as_str()) {
            let mesh = meshes.add(Plane3d::new(Vec3::Z, size / 2.0));
            let material = materials.add(StandardMaterial {
                base_color: placeholder_color_for_bone(&bone_def.name),
                unlit: true,
                double_sided: true,
                cull_mode: None,
                ..default()
            });
            bone_cmds.insert((Mesh3d(mesh), MeshMaterial3d(material)));
        }

        let bone_entity = bone_cmds.id();

        let parent_entity = bone_def
            .parent
            .as_ref()
            .map(|parent_name| {
                *bone_map
                    .get(parent_name.as_str())
                    .expect("parent bone must be spawned before child (topological sort)")
            })
            .unwrap_or(billboard_id);

        commands.entity(parent_entity).add_child(bone_entity);
        bone_map.insert(bone_def.name.clone(), bone_entity);
    }

    bone_map
}

/// Builds a `Transform` from a bone definition, using z_order from slot lookup.
fn bone_transform_from_def(
    bone_def: &crate::asset::BoneDef,
    slot_lookup: &HashMap<&str, (f32, Vec2)>,
) -> Transform {
    let z_order = slot_lookup
        .get(bone_def.name.as_str())
        .map(|(z, _)| *z)
        .unwrap_or(0.0);

    Transform {
        translation: Vec3::new(
            bone_def.default_transform.translation.x,
            bone_def.default_transform.translation.y,
            z_order,
        ),
        rotation: Quat::from_rotation_z(bone_def.default_transform.rotation.to_radians()),
        scale: Vec3::new(
            bone_def.default_transform.scale.x,
            bone_def.default_transform.scale.y,
            1.0,
        ),
    }
}

/// Builds a lookup from bone name to (z_order, attachment size) from the rig's slots and default skin.
fn build_slot_lookup(rig: &SpriteRigAsset) -> HashMap<&str, (f32, Vec2)> {
    let default_skin = rig.skins.get("default");
    let mut lookup = HashMap::new();

    for slot in &rig.slots {
        let size = default_skin
            .and_then(|skin| skin.get(&slot.default_attachment))
            .map(|att| att.size)
            .unwrap_or(Vec2::splat(1.0));

        lookup.insert(slot.bone.as_str(), (slot.z_order, size));
    }

    lookup
}

/// Returns a placeholder color based on bone name keywords.
fn placeholder_color_for_bone(name: &str) -> Color {
    let lower = name.to_lowercase();
    if lower.contains("torso") || lower.contains("chest") || lower.contains("body") {
        Color::srgb(0.2, 0.3, 0.8)
    } else if lower.contains("head") {
        Color::srgb(0.9, 0.8, 0.6)
    } else if lower.contains("arm") || lower.contains("hand") {
        Color::srgb(0.2, 0.7, 0.3)
    } else if lower.contains("leg") || lower.contains("foot") {
        Color::srgb(0.5, 0.3, 0.1)
    } else {
        Color::srgb(0.5, 0.5, 0.5)
    }
}

/// Topological sort: parents before children. Simple iterative approach.
fn topological_sort_bones(bones: &[crate::asset::BoneDef]) -> Vec<&crate::asset::BoneDef> {
    let mut sorted: Vec<&crate::asset::BoneDef> = Vec::with_capacity(bones.len());
    let mut added: std::collections::HashSet<&str> = std::collections::HashSet::new();

    // Iteratively add bones whose parent is already added (or have no parent).
    loop {
        let prev_len = sorted.len();
        for bone in bones {
            if added.contains(bone.name.as_str()) {
                continue; // already sorted
            }
            let parent_satisfied = bone
                .parent
                .as_ref()
                .map_or(true, |p| added.contains(p.as_str()));
            if parent_satisfied {
                sorted.push(bone);
                added.insert(&bone.name);
            }
        }
        if sorted.len() == bones.len() {
            break;
        }
        debug_assert!(
            sorted.len() > prev_len,
            "Bone hierarchy contains a cycle or references a missing parent"
        );
    }

    sorted
}

/// Rotates `RigBillboard` entities to face the camera (Y-axis locked).
///
/// Uses atan2-based Y rotation to avoid the degenerate case of `from_rotation_arc`
/// when the camera direction is anti-parallel to +Z (common with rear-facing cameras).
pub fn billboard_rigs_face_camera(
    camera_query: Query<&GlobalTransform, With<Camera3d>>,
    mut billboard_query: Query<(&GlobalTransform, &mut Transform, &ChildOf), With<RigBillboard>>,
    parent_query: Query<&GlobalTransform, Without<RigBillboard>>,
) {
    let Ok(camera_gt) = camera_query.single() else {
        // Camera not yet spawned during early frames
        return;
    };
    let camera_pos = camera_gt.translation();

    for (global_transform, mut transform, child_of) in &mut billboard_query {
        let billboard_pos = global_transform.translation();
        let direction = (camera_pos - billboard_pos).with_y(0.0);
        if direction.length_squared() < 0.001 {
            continue; // degenerate direction when camera is directly above
        }
        let world_rotation = Quat::from_rotation_y(direction.x.atan2(direction.z));
        let parent_rotation = parent_query
            .get(child_of.parent())
            .map(|gt| gt.to_scale_rotation_translation().1)
            .unwrap_or(Quat::IDENTITY);
        transform.rotation = parent_rotation.inverse() * world_rotation;
    }
}

/// Updates `Facing` based on horizontal velocity.
pub fn update_facing_from_velocity(
    mut query: Query<(&mut Facing, &LinearVelocity), With<CharacterMarker>>,
) {
    for (mut facing, velocity) in &mut query {
        if velocity.x > 0.1 {
            facing.set_if_neq(Facing::Right);
        } else if velocity.x < -0.1 {
            facing.set_if_neq(Facing::Left);
        }
    }
}

/// Mirrors the rig billboard horizontally when `Facing` changes.
pub fn apply_facing_to_rig(
    changed_query: Query<(Entity, &Facing), Changed<Facing>>,
    children_query: Query<&Children>,
    mut billboard_query: Query<&mut Transform, With<RigBillboard>>,
) {
    for (entity, facing) in &changed_query {
        let Ok(children) = children_query.get(entity) else {
            continue; // children not yet propagated on the frame Facing is first inserted
        };
        for child in children.iter() {
            if let Ok(mut transform) = billboard_query.get_mut(child) {
                transform.scale.x = match facing {
                    Facing::Right => 1.0,
                    Facing::Left => -1.0,
                };
            }
        }
    }
}
