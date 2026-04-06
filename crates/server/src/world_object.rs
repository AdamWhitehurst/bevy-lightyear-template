use std::collections::HashSet;

use avian3d::prelude::*;
use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use lightyear::prelude::*;
use protocol::map::MapInstanceId;
use protocol::vox_model::{VoxModelAsset, VoxModelRegistry};
use protocol::world_object::{apply_object_components, VisualKind, WorldObjectDef, WorldObjectId};

/// Spawns a world object entity on the server.
///
/// Lightyear replicates it to all clients on the same map via the room system.
/// `MapInstanceId` triggers `on_map_instance_id_added`, which automatically adds
/// the entity to the correct Lightyear room.
///
/// All gameplay components (Position, RigidBody, CollisionLayers, ObjectCategory,
/// VisualKind, etc.) come from the definition's reflected components.
///
/// Collider priority: vox trimesh (accurate to model shape) is preferred.
/// `ColliderConstructor` from RON is used as a fallback when no vox mesh is available.
/// When vox trimesh is used, `ColliderConstructor` is filtered out of the applied
/// components to prevent Avian from overwriting the trimesh collider.
pub fn spawn_world_object(
    commands: &mut Commands,
    id: WorldObjectId,
    def: &WorldObjectDef,
    map_id: MapInstanceId,
    type_registry: &AppTypeRegistry,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    meshes: &Assets<Mesh>,
) -> Entity {
    let entity = commands
        .spawn((
            id,
            Rotation::default(),
            map_id,
            Replicate::to_clients(NetworkTarget::All),
            NetworkVisibility,
        ))
        .id();

    let vox_collider = vox_trimesh_collider(def, vox_registry, vox_assets, meshes);
    let use_vox_collider = vox_collider.is_some();

    let components = clone_def_components(def, use_vox_collider);
    apply_object_components(commands, entity, components, type_registry.0.clone());

    if let Some(collider) = vox_collider {
        commands.entity(entity).insert(collider);
    }

    entity
}

/// Transforms an entity from its current def to a source def by diffing components.
///
/// Removes components present in `current_def` but absent from `source_def`.
/// Inserts/overwrites components from `source_def`.
/// Handles vox collider swap: builds trimesh from source if it has a Vox visual.
pub fn apply_transformation(
    commands: &mut Commands,
    entity: Entity,
    current_def: &WorldObjectDef,
    source_def: &WorldObjectDef,
    type_registry: &AppTypeRegistry,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    meshes: &Assets<Mesh>,
) {
    let source_type_paths: HashSet<&str> = source_def
        .components
        .iter()
        .map(|c| c.reflect_type_path())
        .collect();

    remove_absent_components(
        commands,
        entity,
        current_def,
        &source_type_paths,
        type_registry,
    );

    let vox_collider = vox_trimesh_collider(source_def, vox_registry, vox_assets, meshes);
    let use_vox_collider = vox_collider.is_some();
    let components = clone_def_components(source_def, use_vox_collider);
    apply_object_components(commands, entity, components, type_registry.0.clone());

    if let Some(collider) = vox_collider {
        commands.entity(entity).insert(collider);
    }
}

/// Removes reflected components from `entity` that are in `current_def` but not in `keep_paths`.
fn remove_absent_components(
    commands: &mut Commands,
    entity: Entity,
    current_def: &WorldObjectDef,
    keep_paths: &HashSet<&str>,
    type_registry: &AppTypeRegistry,
) {
    let registry = type_registry.read();
    for component in &current_def.components {
        let path = component.reflect_type_path();
        if keep_paths.contains(path) {
            continue;
        }
        let Some(registration) = registry.get_with_type_path(path) else {
            continue;
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            continue;
        };
        let reflect_component = reflect_component.clone();
        commands.queue(move |world: &mut World| {
            if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
                reflect_component.remove(&mut entity_mut);
            }
        });
    }
}

/// Clones the definition's reflected components for insertion.
///
/// When `filter_collider_constructor` is true, `ColliderConstructor` is excluded
/// because a vox trimesh collider takes priority.
pub(crate) fn clone_def_components(
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
    let vox_path = def
        .components
        .iter()
        // try to find a VisualKind Component, ignoring failed downcasts because they are other components
        // No need to trace every component
        .find_map(|c| match c.try_downcast_ref::<VisualKind>()? {
            VisualKind::Vox(path) => Some(path.as_str()),
            _ => None,
        })?;

    let mesh = vox_registry.get_lod0_mesh(vox_path, vox_assets, meshes)?;
    Collider::trimesh_from_mesh(mesh)
}
