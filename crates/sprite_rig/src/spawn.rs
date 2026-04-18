use std::collections::HashMap;

use avian3d::prelude::{LinearVelocity, Position, Rotation};
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, TextureAtlasBuilder};
use bevy::light::NotShadowCaster;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::ExtendedMaterial;
use bevy::prelude::*;
use lightyear::prelude::*;
use protocol::app_state::TrackedAssets;
use protocol::billboard::shadow_only_material::{ShadowOnlyExt, ShadowOnlyMaterial};
use protocol::billboard::sprite_rig_material::{SpriteRigBillboardExt, SpriteRigMaterial};
use protocol::{transform_from_physics, CharacterMarker, CharacterType};

use crate::asset::{SpriteAnchorDef, SpriteRigAsset};
use crate::shadow_twin::ShadowTwinOf;
use crate::{asset::SpriteAnimSetAsset, RigRegistry};

/// Reference to the rig asset for this character. Triggers rig spawning.
#[derive(Component)]
pub struct SpriteRig(pub Handle<SpriteRigAsset>);

/// Reference to the animset for this character.
#[derive(Component)]
pub struct AnimSetRef(pub Handle<SpriteAnimSetAsset>);

/// Maps bone names to their spawned joint entities.
#[derive(Component, Default)]
pub struct BoneEntities(pub HashMap<String, Entity>);

/// Horizontal facing direction derived from velocity.
#[derive(Component, PartialEq, Clone, Copy)]
pub enum Facing {
    Left,
    Right,
}

/// Marker for the joint root child entity that parents all root bone entities.
#[derive(Component)]
pub struct JointRoot;

/// Per-bone slot data extracted from the rig's default skin.
struct SlotInfo {
    z_order: f32,
    size: Vec2,
    anchor: SpriteAnchorDef,
    image_path: String,
}

/// Cached GPU assets for a rig type, shared across all characters using that rig.
struct RigMeshAssets {
    mesh: Handle<Mesh>,
    inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
    material: Handle<SpriteRigMaterial>,
    shadow_parts: Vec<ShadowPartAssets>,
}

/// Shared mesh and material for a single bone's shadow twin quad, keyed by bone name
/// so each character's twin can resolve to its own joint entity at spawn time.
struct ShadowPartAssets {
    bone_name: String,
    mesh: Handle<Mesh>,
    material: Handle<ShadowOnlyMaterial>,
}

/// Cache of rig mesh assets keyed by rig asset handle ID.
#[derive(Resource, Default)]
pub struct RigMeshCache(HashMap<AssetId<SpriteRigAsset>, RigMeshAssets>);

/// Strong handles to sprite images loaded from rig skins, keyed by asset path.
#[derive(Resource, Default)]
pub struct SpriteImageHandles(pub HashMap<String, Handle<Image>>);

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

/// Loads sprite images referenced by rig skins and tracks them for `AppState::Ready`.
pub fn load_rig_sprite_images(
    registry: Res<RigRegistry>,
    rig_assets: Res<Assets<SpriteRigAsset>>,
    asset_server: Res<AssetServer>,
    mut sprite_images: ResMut<SpriteImageHandles>,
    mut tracked: ResMut<TrackedAssets>,
) {
    for entry in registry.entries.values() {
        let Some(rig) = rig_assets.get(&entry.rig_handle) else {
            trace!("Rig asset not yet loaded, will retry next frame");
            continue;
        };
        let Some(default_skin) = rig.skins.get("default") else {
            continue;
        };
        for attachment in default_skin.values() {
            if sprite_images.0.contains_key(&attachment.image) {
                continue;
            }
            let handle = asset_server.load::<Image>(&attachment.image);
            tracked.add(handle.clone());
            sprite_images.0.insert(attachment.image.clone(), handle);
        }
    }
}

/// Spawns joint hierarchy and a single skinned mesh when `SpriteRig` is added.
pub fn spawn_sprite_rigs(
    mut commands: Commands,
    query: Query<(Entity, &SpriteRig, Option<&Position>, Option<&Rotation>), Added<SpriteRig>>,
    rig_assets: Res<Assets<SpriteRigAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<SpriteRigMaterial>>,
    mut shadow_materials: ResMut<Assets<ShadowOnlyMaterial>>,
    mut bindpose_assets: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    mut rig_mesh_cache: ResMut<RigMeshCache>,
    sprite_images: Res<SpriteImageHandles>,
) {
    for (entity, sprite_rig, pos, rot) in &query {
        let rig = rig_assets
            .get(&sprite_rig.0)
            .expect("SpriteRigAsset must be loaded before gameplay (AppState::Ready)");

        let slot_lookup = build_slot_lookup(rig);
        let sorted_bones = topological_sort_bones(&rig.bones);

        let cached = rig_mesh_cache
            .0
            .entry(sprite_rig.0.id())
            .or_insert_with(|| {
                build_rig_mesh_assets(
                    &sorted_bones,
                    &slot_lookup,
                    &mut meshes,
                    &mut images,
                    &mut materials,
                    &mut shadow_materials,
                    &mut bindpose_assets,
                    &sprite_images,
                )
            });

        let mesh_handle = cached.mesh.clone();
        let bindpose_handle = cached.inverse_bindposes.clone();
        let material_handle = cached.material.clone();
        let shadow_parts: Vec<(String, Handle<Mesh>, Handle<ShadowOnlyMaterial>)> = cached
            .shadow_parts
            .iter()
            .map(|p| (p.bone_name.clone(), p.mesh.clone(), p.material.clone()))
            .collect();

        let joint_root_id = commands
            .spawn((JointRoot, Name::new("JointRoot"), Transform::default()))
            .id();
        // Insert Transform matching Position so children have a parent with
        // GlobalTransform. PhysicsTransformPlugin is disabled; lightyear's
        // add_transform only runs in PostUpdate, after children are attached.
        let transform = transform_from_physics(pos, rot);
        commands
            .entity(entity)
            .insert((transform, Visibility::default()))
            .add_child(joint_root_id);

        let (bone_map, joint_entities) =
            spawn_joints(&mut commands, joint_root_id, &sorted_bones, &slot_lookup);

        commands.entity(entity).with_child((
            Name::new("SkinnedMesh"),
            Mesh3d(mesh_handle),
            MeshMaterial3d(material_handle),
            SkinnedMesh {
                inverse_bindposes: bindpose_handle,
                joints: joint_entities,
            },
            Transform::default(),
            NotShadowCaster,
        ));

        spawn_shadow_twins(&mut commands, entity, &bone_map, &shadow_parts);

        commands.entity(entity).insert(BoneEntities(bone_map));
    }
}

/// Spawns one shadow twin quad per visible bone as a child of the character entity.
/// Parenting ensures twins despawn with the character (including when a lightyear
/// Room change or disconnect removes the replicated rig). The twins live on the
/// default render layer so `queue_shadows` includes them in the camera's shadow
/// pass; their material discards every main-pass fragment, keeping them visually
/// invisible while still writing alpha-masked depth into the shadow map.
fn spawn_shadow_twins(
    commands: &mut Commands,
    character_entity: Entity,
    bone_map: &HashMap<String, Entity>,
    shadow_parts: &[(String, Handle<Mesh>, Handle<ShadowOnlyMaterial>)],
) {
    for (bone_name, mesh, material) in shadow_parts {
        let joint_entity = *bone_map
            .get(bone_name.as_str())
            .expect("shadow part bone must exist in bone_map");
        commands.entity(character_entity).with_child((
            Name::new(format!("ShadowTwin[{bone_name}]")),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::IDENTITY,
            Visibility::default(),
            ShadowTwinOf(joint_entity),
        ));
    }
}

/// Builds and caches the mesh, inverse bind poses, material, and per-bone shadow
/// assets for a rig type.
fn build_rig_mesh_assets(
    sorted_bones: &[&crate::asset::BoneDef],
    slot_lookup: &HashMap<&str, SlotInfo>,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<SpriteRigMaterial>,
    shadow_materials: &mut Assets<ShadowOnlyMaterial>,
    bindpose_assets: &mut Assets<SkinnedMeshInverseBindposes>,
    sprite_images: &SpriteImageHandles,
) -> RigMeshAssets {
    let bone_index_map = build_bone_index_map(sorted_bones);

    let (atlas_texture, uv_rects) =
        build_texture_atlas(sorted_bones, slot_lookup, images, sprite_images);

    let mesh = build_rig_mesh(sorted_bones, slot_lookup, &bone_index_map, &uv_rects);
    let inverse_bindposes = bindpose_assets.add(vec![Mat4::IDENTITY; sorted_bones.len()]);

    let atlas_handle = images.add(atlas_texture);
    let material = materials.add(SpriteRigMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(atlas_handle),
            unlit: true,
            double_sided: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Blend,
            ..default()
        },
        extension: SpriteRigBillboardExt {},
    });

    let shadow_parts = build_shadow_part_assets(
        sorted_bones,
        slot_lookup,
        meshes,
        shadow_materials,
        sprite_images,
    );

    RigMeshAssets {
        mesh: meshes.add(mesh),
        inverse_bindposes,
        material,
        shadow_parts,
    }
}

/// Builds one shadow quad mesh and one alpha-masked `ShadowOnlyMaterial` per
/// visible bone. The material's main-pass fragment discards, so the quad is
/// invisible to the camera; its prepass and shadow pass honor the sprite
/// texture's alpha mask to produce the silhouette shadow.
fn build_shadow_part_assets(
    sorted_bones: &[&crate::asset::BoneDef],
    slot_lookup: &HashMap<&str, SlotInfo>,
    meshes: &mut Assets<Mesh>,
    shadow_materials: &mut Assets<ShadowOnlyMaterial>,
    sprite_images: &SpriteImageHandles,
) -> Vec<ShadowPartAssets> {
    let visible_bones = collect_visible_bones(sorted_bones, slot_lookup);
    let mut parts = Vec::with_capacity(visible_bones.len());

    for (bone_def, slot_info) in &visible_bones {
        let texture_handle = sprite_images
            .0
            .get(&slot_info.image_path)
            .unwrap_or_else(|| {
                panic!(
                    "missing sprite image handle for shadow twin '{}'",
                    slot_info.image_path
                )
            })
            .clone();

        let mesh = meshes.add(Rectangle::new(slot_info.size.x, slot_info.size.y));
        let material = shadow_materials.add(ExtendedMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(texture_handle),
                alpha_mode: AlphaMode::Mask(0.5),
                double_sided: true,
                cull_mode: None,
                ..default()
            },
            extension: ShadowOnlyExt {},
        });

        parts.push(ShadowPartAssets {
            bone_name: bone_def.name.clone(),
            mesh,
            material,
        });
    }

    parts
}

/// Packs all visible bone images into a single texture atlas.
///
/// Returns the atlas image (with nearest sampling) and a map of bone name to normalized UV rect.
fn build_texture_atlas<'a>(
    sorted_bones: &[&'a crate::asset::BoneDef],
    slot_lookup: &HashMap<&str, SlotInfo>,
    images: &Assets<Image>,
    sprite_images: &SpriteImageHandles,
) -> (Image, HashMap<&'a str, Rect>) {
    let mut builder = TextureAtlasBuilder::default();
    builder.padding(UVec2::new(2, 2));

    let visible_bones = collect_visible_bones(sorted_bones, slot_lookup);
    let mut bone_image_ids: Vec<(&str, AssetId<Image>)> = Vec::new();

    for (bone_def, slot_info) in &visible_bones {
        let handle = sprite_images
            .0
            .get(&slot_info.image_path)
            .unwrap_or_else(|| {
                panic!("missing sprite image handle for '{}'", slot_info.image_path)
            });
        let image = images
            .get(handle)
            .unwrap_or_else(|| panic!("sprite image not loaded for '{}'", slot_info.image_path));
        let asset_id = handle.id();
        builder.add_texture(Some(asset_id), image);
        bone_image_ids.push((bone_def.name.as_str(), asset_id));
    }

    let (layout, sources, mut atlas_image) = builder.build().expect("texture atlas build failed");
    atlas_image.sampler = ImageSampler::nearest();

    let mut uv_rects = HashMap::new();
    for (bone_name, image_id) in &bone_image_ids {
        let uv_rect = sources
            .uv_rect(&layout, *image_id)
            .unwrap_or_else(|| panic!("missing UV rect for bone '{bone_name}'"));
        uv_rects.insert(*bone_name, uv_rect);
    }

    (atlas_image, uv_rects)
}

/// Maps bone name to its index in topological sort order.
fn build_bone_index_map<'a>(sorted_bones: &[&'a crate::asset::BoneDef]) -> HashMap<&'a str, u16> {
    sorted_bones
        .iter()
        .enumerate()
        .map(|(i, bone)| {
            let idx = u16::try_from(i).expect("bone count exceeds u16::MAX");
            (bone.name.as_str(), idx)
        })
        .collect()
}

/// Builds a single merged mesh with one quad per visible bone, skinned to its joint.
fn build_rig_mesh(
    sorted_bones: &[&crate::asset::BoneDef],
    slot_lookup: &HashMap<&str, SlotInfo>,
    bone_index_map: &HashMap<&str, u16>,
    uv_rects: &HashMap<&str, Rect>,
) -> Mesh {
    let mut visible_bones = collect_visible_bones(sorted_bones, slot_lookup);
    visible_bones.sort_by(|a, b| {
        a.1.z_order
            .partial_cmp(&b.1.z_order)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut joint_indices: Vec<[u16; 4]> = Vec::new();
    let mut joint_weights: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for (bone_def, slot_info) in &visible_bones {
        let joint_idx = *bone_index_map
            .get(bone_def.name.as_str())
            .expect("visible bone must exist in bone_index_map");
        let base_vertex = u32::try_from(positions.len()).expect("vertex count exceeds u32::MAX");

        let uv_rect = uv_rects
            .get(bone_def.name.as_str())
            .unwrap_or_else(|| panic!("missing UV rect for bone '{}'", bone_def.name));

        append_quad_vertices(
            slot_info.size,
            joint_idx,
            *uv_rect,
            &slot_info.anchor,
            &mut positions,
            &mut normals,
            &mut uvs,
            &mut joint_indices,
            &mut joint_weights,
        );
        append_quad_indices(base_vertex, &mut indices);
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_JOINT_INDEX,
        VertexAttributeValues::Uint16x4(joint_indices),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, joint_weights)
    .with_inserted_indices(Indices::U32(indices))
}

/// Collects bones that have a slot entry, returning (bone_def, slot_info) pairs.
fn collect_visible_bones<'a, 'b>(
    sorted_bones: &[&'a crate::asset::BoneDef],
    slot_lookup: &'b HashMap<&str, SlotInfo>,
) -> Vec<(&'a crate::asset::BoneDef, &'b SlotInfo)> {
    sorted_bones
        .iter()
        .filter_map(|bone| {
            slot_lookup
                .get(bone.name.as_str())
                .map(|info| (*bone, info))
        })
        .collect()
}

/// Appends 4 vertices for a Z-facing quad with per-bone UV rect and anchor offset.
fn append_quad_vertices(
    size: Vec2,
    joint_idx: u16,
    uv_rect: Rect,
    anchor: &SpriteAnchorDef,
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    joint_indices: &mut Vec<[u16; 4]>,
    joint_weights: &mut Vec<[f32; 4]>,
) {
    let hx = size.x / 2.0;
    let hy = size.y / 2.0;
    let y_offset = anchor_y_offset(anchor, size);

    positions.extend_from_slice(&[
        [-hx, -hy + y_offset, 0.0],
        [hx, -hy + y_offset, 0.0],
        [hx, hy + y_offset, 0.0],
        [-hx, hy + y_offset, 0.0],
    ]);
    normals.extend_from_slice(&[[0.0, 0.0, 1.0]; 4]);
    uvs.extend_from_slice(&[
        [uv_rect.min.x, uv_rect.max.y],
        [uv_rect.max.x, uv_rect.max.y],
        [uv_rect.max.x, uv_rect.min.y],
        [uv_rect.min.x, uv_rect.min.y],
    ]);

    let ji = [joint_idx, 0, 0, 0];
    let jw = [1.0, 0.0, 0.0, 0.0];
    joint_indices.extend_from_slice(&[ji; 4]);
    joint_weights.extend_from_slice(&[jw; 4]);
}

/// Computes the Y offset for a quad based on anchor type.
fn anchor_y_offset(anchor: &SpriteAnchorDef, size: Vec2) -> f32 {
    match anchor {
        SpriteAnchorDef::Center => 0.0,
        SpriteAnchorDef::TopCenter => -size.y / 2.0,
        SpriteAnchorDef::BottomCenter => size.y / 2.0,
    }
}

/// Appends 6 indices (2 triangles) for a quad starting at `base_vertex`.
fn append_quad_indices(base_vertex: u32, indices: &mut Vec<u32>) {
    indices.extend_from_slice(&[
        base_vertex,
        base_vertex + 1,
        base_vertex + 2,
        base_vertex,
        base_vertex + 2,
        base_vertex + 3,
    ]);
}

/// Spawns transform-only joint entities in topological order, returning the bone map and
/// ordered joint entity list matching the inverse bind pose array.
fn spawn_joints(
    commands: &mut Commands,
    joint_root_id: Entity,
    sorted_bones: &[&crate::asset::BoneDef],
    slot_lookup: &HashMap<&str, SlotInfo>,
) -> (HashMap<String, Entity>, Vec<Entity>) {
    let mut bone_map = HashMap::<String, Entity>::new();
    let mut joint_entities = Vec::with_capacity(sorted_bones.len());

    for bone_def in sorted_bones {
        let slot_info = slot_lookup.get(bone_def.name.as_str());
        let transform = bone_transform_from_def(bone_def, slot_info);

        let joint_entity = commands
            .spawn((Name::new(bone_def.name.clone()), transform))
            .id();

        let parent_entity = bone_def
            .parent
            .as_ref()
            .map(|parent_name| {
                *bone_map
                    .get(parent_name.as_str())
                    .expect("parent bone must be spawned before child (topological sort)")
            })
            .unwrap_or(joint_root_id);

        commands.entity(parent_entity).add_child(joint_entity);
        bone_map.insert(bone_def.name.clone(), joint_entity);
        joint_entities.push(joint_entity);
    }

    (bone_map, joint_entities)
}

/// Builds a `Transform` from a bone definition, using z_order from slot info.
fn bone_transform_from_def(
    bone_def: &crate::asset::BoneDef,
    slot_info: Option<&SlotInfo>,
) -> Transform {
    let z_order = slot_info.map(|s| s.z_order).unwrap_or(0.0);

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

/// Builds a lookup from bone name to slot info from the rig's slots and default skin.
fn build_slot_lookup(rig: &SpriteRigAsset) -> HashMap<&str, SlotInfo> {
    let default_skin = rig.skins.get("default");
    let mut lookup = HashMap::new();

    for slot in &rig.slots {
        let attachment = default_skin.and_then(|skin| skin.get(&slot.default_attachment));

        let info = SlotInfo {
            z_order: slot.z_order,
            size: attachment.map(|a| a.size).unwrap_or(Vec2::splat(1.0)),
            anchor: attachment.map(|a| a.anchor.clone()).unwrap_or_default(),
            image_path: attachment.map(|a| a.image.clone()).unwrap_or_default(),
        };

        lookup.insert(slot.bone.as_str(), info);
    }

    lookup
}

/// Topological sort: parents before children. Simple iterative approach.
fn topological_sort_bones(bones: &[crate::asset::BoneDef]) -> Vec<&crate::asset::BoneDef> {
    let mut sorted: Vec<&crate::asset::BoneDef> = Vec::with_capacity(bones.len());
    let mut added: std::collections::HashSet<&str> = std::collections::HashSet::new();

    loop {
        let prev_len = sorted.len();
        for bone in bones {
            if added.contains(bone.name.as_str()) {
                continue;
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

/// Updates `Facing` based on horizontal velocity projected onto camera's right axis.
pub fn update_facing_from_velocity(
    camera_query: Query<&Transform, With<Camera3d>>,
    mut query: Query<(&mut Facing, &LinearVelocity), With<CharacterMarker>>,
) {
    let Ok(camera_transform) = camera_query.single() else {
        return;
    };
    let camera_right = camera_transform.right().as_vec3();
    let camera_right_xz = Vec3::new(camera_right.x, 0.0, camera_right.z);

    for (mut facing, velocity) in &mut query {
        let lateral = velocity.dot(camera_right_xz);
        if lateral > 0.1 {
            facing.set_if_neq(Facing::Left);
        } else if lateral < -0.1 {
            facing.set_if_neq(Facing::Right);
        }
    }
}

/// Rotates `JointRoot` entities to face the camera (Y-axis locked).
///
/// This keeps the 2D bone hierarchy layout in the camera-facing plane so bone
/// offsets (arm to the side, head above, etc.) project correctly on screen.
/// The GPU billboard shader handles per-quad screen-plane orientation on top.
pub fn billboard_joint_roots(
    camera_query: Query<&GlobalTransform, With<Camera3d>>,
    mut joint_query: Query<(&GlobalTransform, &mut Transform, &ChildOf), With<JointRoot>>,
    parent_query: Query<&GlobalTransform, Without<JointRoot>>,
) {
    let Ok(camera_gt) = camera_query.single() else {
        trace!("Camera not yet spawned during early frames");
        return;
    };
    let camera_pos = camera_gt.translation();

    for (global_transform, mut transform, child_of) in &mut joint_query {
        let pos = global_transform.translation();
        let direction = (camera_pos - pos).with_y(0.0);
        if direction.length_squared() < 0.001 {
            continue;
        }
        let world_rotation = Quat::from_rotation_y(direction.x.atan2(direction.z));
        let parent_rotation = parent_query
            .get(child_of.parent())
            .map(|gt| gt.to_scale_rotation_translation().1)
            .unwrap_or(Quat::IDENTITY);
        transform.rotation = parent_rotation.inverse() * world_rotation;
    }
}

/// Mirrors the joint root horizontally when `Facing` changes.
pub fn apply_facing_to_rig(
    changed_query: Query<(Entity, &Facing), Changed<Facing>>,
    children_query: Query<&Children>,
    mut billboard_query: Query<&mut Transform, With<JointRoot>>,
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
