use avian3d::prelude::{Collider, ColliderConstructor};
use bevy::prelude::*;
use lightyear::prelude::Replicated;
use protocol::vox_model::{VoxModelAsset, VoxModelRegistry};
use protocol::world_object::{
    apply_object_components, VisualKind, WorldObjectDef, WorldObjectDefRegistry, WorldObjectId,
};
use protocol::{MapInstanceId, MapRegistry};

/// Shared PBR material for all vox model meshes.
///
/// Vertex colors from the vox model are multiplied with `base_color` (white),
/// so the palette colors pass through unmodified.
#[derive(Resource)]
pub struct DefaultVoxModelMaterial(pub Handle<StandardMaterial>);

/// Creates the shared vox model material at startup.
pub fn init_default_vox_model_material(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let handle = materials.add(StandardMaterial::default());
    commands.insert_resource(DefaultVoxModelMaterial(handle));
}

/// Reacts when Lightyear replicates a world object entity to this client.
///
/// Mirrors the server's `spawn_world_object` pattern: if a vox model exists,
/// generates a trimesh collider and filters out the RON `ColliderConstructor`
/// to prevent avian from overwriting it.
pub fn on_world_object_replicated(
    query: Query<(Entity, &WorldObjectId), Added<Replicated>>,
    registry: Res<WorldObjectDefRegistry>,
    map_registry: Res<MapRegistry>,
    map_id_query: Query<&MapInstanceId>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
    type_registry: Res<AppTypeRegistry>,
    default_material: Res<DefaultVoxModelMaterial>,
    mut commands: Commands,
) {
    for (entity, id) in &query {
        if let Ok(entity_mid) = map_id_query.get(entity) {
            if !map_registry.0.contains_key(entity_mid) {
                trace!("Despawning stale world object {entity:?} from map {entity_mid:?}");
                commands.entity(entity).despawn();
                continue;
            }
        }

        let Some(def) = registry.get(id) else {
            warn!("Replicated world object has unknown id: {:?}", id.0);
            continue;
        };

        let vox_collider = vox_trimesh_collider(def, &vox_registry, &vox_assets, &meshes);
        let has_vox_collider = vox_collider.is_some();

        let components = clone_def_components(def, has_vox_collider);
        apply_object_components(&mut commands, entity, components, type_registry.0.clone());

        if let Some(collider) = vox_collider {
            commands.entity(entity).insert(collider);
        }

        attach_visual(
            &mut commands,
            entity,
            def,
            &vox_registry,
            &vox_assets,
            &default_material,
        );
    }
}

/// Clones the definition's reflected components for insertion.
///
/// When `filter_collider_constructor` is true, `ColliderConstructor` is excluded
/// because a vox trimesh collider takes priority.
fn clone_def_components(
    def: &WorldObjectDef,
    filter_collider_constructor: bool,
) -> Vec<Box<dyn bevy::reflect::PartialReflect>> {
    def.components
        .iter()
        .filter(|c| {
            !filter_collider_constructor || c.try_downcast_ref::<ColliderConstructor>().is_none()
        })
        .map(|c| {
            c.reflect_clone()
                .expect("world object component must be cloneable")
                .into_partial_reflect()
        })
        .collect()
}

/// Derives a trimesh `Collider` from the vox model mesh referenced by `VisualKind::Vox`.
fn vox_trimesh_collider(
    def: &WorldObjectDef,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    meshes: &Assets<Mesh>,
) -> Option<Collider> {
    let vox_path =
        def.components
            .iter()
            .find_map(|c| match c.try_downcast_ref::<VisualKind>()? {
                VisualKind::Vox(path) => Some(path.as_str()),
                _ => None,
            })?;

    let mesh = vox_registry.get_lod0_mesh(vox_path, vox_assets, meshes)?;
    Collider::trimesh_from_mesh(mesh)
}

/// Attaches the vox mesh as a child entity if `VisualKind::Vox` is present.
fn attach_visual(
    commands: &mut Commands,
    entity: Entity,
    def: &WorldObjectDef,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    default_material: &DefaultVoxModelMaterial,
) {
    let visual_kind = def
        .components
        .iter()
        .find_map(|c| c.try_downcast_ref::<VisualKind>());

    match visual_kind {
        Some(VisualKind::Vox(path)) => {
            attach_vox_mesh(
                commands,
                entity,
                path,
                vox_registry,
                vox_assets,
                default_material,
            );
        }
        _ => {
            trace!("World object entity {entity:?} has no Vox visual, skipping mesh attachment");
        }
    }
}

/// Rebuilds visuals and collider when VisualKind changes via replication (e.g. tree→stump).
pub fn on_visual_kind_changed(
    mut commands: Commands,
    query: Query<(Entity, &VisualKind), Changed<VisualKind>>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
    default_material: Res<DefaultVoxModelMaterial>,
    children_query: Query<&Children>,
) {
    for (entity, visual) in &query {
        // Despawn old visual children
        if let Ok(children) = children_query.get(entity) {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }

        // Remove old collider and rebuild from new visual
        commands.entity(entity).remove::<Collider>();
        if let VisualKind::Vox(path) = visual {
            if let Some(collider) =
                vox_trimesh_collider_from_path(path, &vox_registry, &vox_assets, &meshes)
            {
                commands.entity(entity).insert(collider);
            }
            attach_vox_mesh(
                &mut commands,
                entity,
                path,
                &vox_registry,
                &vox_assets,
                &default_material,
            );
        } else {
            trace!("Entity {entity:?} visual changed to non-Vox, no mesh to attach");
        }
    }
}

/// Builds a trimesh collider from a vox model path.
fn vox_trimesh_collider_from_path(
    vox_path: &str,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    meshes: &Assets<Mesh>,
) -> Option<Collider> {
    let mesh = vox_registry.get_lod0_mesh(vox_path, vox_assets, meshes)?;
    Collider::trimesh_from_mesh(mesh)
}

/// Attaches the LOD 0 (full-resolution) vox mesh as a child of the world object entity.
fn attach_vox_mesh(
    commands: &mut Commands,
    entity: Entity,
    vox_path: &str,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    default_material: &DefaultVoxModelMaterial,
) {
    let Some(asset_handle) = vox_registry.get(vox_path) else {
        warn!("Vox model not found in registry: {vox_path}");
        return;
    };
    let Some(asset) = vox_assets.get(asset_handle) else {
        warn!("VoxModelAsset not yet loaded: {vox_path}");
        return;
    };
    let Some(mesh_handle) = asset.lod_meshes.first() else {
        warn!("VoxModelAsset has no LOD meshes: {vox_path}");
        return;
    };

    commands.entity(entity).with_child((
        Mesh3d(mesh_handle.clone()),
        MeshMaterial3d(default_material.0.clone()),
    ));
}
