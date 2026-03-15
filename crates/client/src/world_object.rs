use avian3d::prelude::ColliderConstructor;
use bevy::prelude::*;
use lightyear::prelude::Replicated;
use protocol::world_object::{apply_object_components, WorldObjectDefRegistry, WorldObjectId};

/// Reacts when Lightyear replicates a world object entity to this client.
///
/// Attaches all reflected gameplay components (including `RigidBody`, `CollisionLayers`,
/// `ColliderConstructor`, `ObjectCategory`, `VisualKind`, etc.) from the definition,
/// then inserts a placeholder mesh derived from the collider shape.
pub fn on_world_object_replicated(
    query: Query<(Entity, &WorldObjectId), Added<Replicated>>,
    registry: Res<WorldObjectDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, id) in &query {
        let Some(def) = registry.get(id) else {
            warn!("Replicated world object has unknown id: {:?}", id.0);
            continue;
        };

        // Extract collider from the components vec for the placeholder mesh.
        let collider = def
            .components
            .iter()
            .find_map(|c| c.try_downcast_ref::<ColliderConstructor>().cloned());

        insert_placeholder_mesh(
            &mut commands.entity(entity),
            collider.as_ref(),
            &mut meshes,
            &mut materials,
        );

        let components = def
            .components
            .iter()
            .map(|c| {
                c.reflect_clone()
                    .expect("world object component must be cloneable")
                    .into_partial_reflect()
            })
            .collect();
        apply_object_components(&mut commands, entity, components, type_registry.0.clone());
    }
}

/// Inserts a `Mesh3d` placeholder derived from the collider shape.
///
/// Once the vox loading pipeline is implemented, this will be replaced by the
/// actual visual from `VisualKind`.
fn insert_placeholder_mesh(
    ecmds: &mut EntityCommands,
    collider: Option<&ColliderConstructor>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let Some(mesh) = collider_to_mesh(collider) else {
        return;
    };
    let mesh_handle = meshes.add(mesh);
    let material_handle = materials.add(StandardMaterial {
        base_color: Color::srgb(0.3, 0.6, 0.2),
        ..default()
    });
    ecmds.insert((Mesh3d(mesh_handle), MeshMaterial3d(material_handle)));
}

/// Converts a `ColliderConstructor` into an approximate `Mesh` for visualization.
fn collider_to_mesh(collider: Option<&ColliderConstructor>) -> Option<Mesh> {
    match collider? {
        ColliderConstructor::Sphere { radius } => Some(Sphere::new(*radius).into()),
        ColliderConstructor::Cuboid {
            x_length,
            y_length,
            z_length,
        } => Some(Cuboid::new(*x_length, *y_length, *z_length).into()),
        ColliderConstructor::Cylinder { radius, height } => {
            Some(Cylinder::new(*radius, *height).into())
        }
        ColliderConstructor::Capsule { radius, height } => {
            Some(Capsule3d::new(*radius, *height).into())
        }
        _ => {
            trace!("No placeholder mesh for collider shape");
            None
        }
    }
}
