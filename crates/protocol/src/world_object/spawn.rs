use bevy::prelude::*;
use bevy::reflect::{PartialReflect, TypeRegistryArc};

/// Queues a command to insert all reflected components from a `WorldObjectDef` onto `entity`.
///
/// Must be called via `commands.queue` because `ReflectComponent::insert` requires
/// `EntityWorldMut`, which is only available in command execution.
pub fn apply_object_components(
    commands: &mut Commands,
    entity: Entity,
    components: Vec<Box<dyn PartialReflect>>,
    registry: TypeRegistryArc,
) {
    commands.queue(move |world: &mut World| {
        let registry = registry.read();
        let mut entity_mut = world.entity_mut(entity);
        for component in &components {
            insert_reflected_component(&mut entity_mut, component.as_ref(), &registry);
        }
    });
}

fn insert_reflected_component(
    entity_mut: &mut EntityWorldMut,
    component: &dyn PartialReflect,
    registry: &bevy::reflect::TypeRegistry,
) {
    let type_path = component.reflect_type_path();
    let Some(registration) = registry.get_with_type_path(type_path) else {
        warn!("World object component type not registered: {type_path}");
        return;
    };
    let Some(reflect_component) = registration.data::<ReflectComponent>() else {
        warn!("Type missing #[reflect(Component)]: {type_path}");
        return;
    };
    reflect_component.insert(entity_mut, component, registry);
}
