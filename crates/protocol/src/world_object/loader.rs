use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::reflect::serde::{TypeRegistrationDeserializer, TypedReflectDeserializer};
use bevy::reflect::{PartialReflect, ReflectFromReflect, TypePath, TypeRegistry, TypeRegistryArc};
use serde::de::{DeserializeSeed, Deserializer, MapAccess, Visitor};
use std::fmt;

use super::types::{WorldObjectDef, WorldObjectLoadError};

/// Custom asset loader that uses `TypeRegistry` for reflect-based component deserialization.
#[derive(TypePath)]
pub(super) struct WorldObjectLoader {
    type_registry: TypeRegistryArc,
}

impl FromWorld for WorldObjectLoader {
    fn from_world(world: &mut World) -> Self {
        Self {
            type_registry: world.resource::<AppTypeRegistry>().0.clone(),
        }
    }
}

impl AssetLoader for WorldObjectLoader {
    type Asset = WorldObjectDef;
    type Settings = ();
    type Error = WorldObjectLoadError;

    fn extensions(&self) -> &[&str] {
        &["object.ron"]
    }

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let registry = self.type_registry.read();
        deserialize_world_object(&bytes, &registry)
    }
}

/// Deserializes a `WorldObjectDef` from RON bytes using the given `TypeRegistry`.
///
/// The RON format is a flat map of type paths to component data:
/// ```ron
/// {
///     "protocol::world_object::ObjectCategory": Scenery,
///     "protocol::world_object::VisualKind": Vox("models/trees/tree.vox"),
///     "protocol::Health": (current: 50.0, max: 50.0),
/// }
/// ```
pub fn deserialize_world_object(
    bytes: &[u8],
    registry: &TypeRegistry,
) -> Result<WorldObjectDef, WorldObjectLoadError> {
    let mut deserializer = ron::de::Deserializer::from_bytes(bytes)?;
    let components = ComponentMapDeserializer { registry }.deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(WorldObjectDef { components })
}

struct ComponentMapDeserializer<'a> {
    registry: &'a TypeRegistry,
}

impl<'a, 'de> DeserializeSeed<'de> for ComponentMapDeserializer<'a> {
    type Value = Vec<Box<dyn PartialReflect>>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(ComponentMapVisitor {
            registry: self.registry,
        })
    }
}

struct ComponentMapVisitor<'a> {
    registry: &'a TypeRegistry,
}

impl<'a, 'de> Visitor<'de> for ComponentMapVisitor<'a> {
    type Value = Vec<Box<dyn PartialReflect>>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a map of component type paths to component data")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut components = Vec::new();
        while let Some(registration) =
            map.next_key_seed(TypeRegistrationDeserializer::new(self.registry))?
        {
            let value =
                map.next_value_seed(TypedReflectDeserializer::new(registration, self.registry))?;
            // Attempt to convert the dynamic representation to a concrete type.
            let value = self
                .registry
                .get(registration.type_id())
                .and_then(|tr| tr.data::<ReflectFromReflect>())
                .and_then(|fr| fr.from_reflect(value.as_partial_reflect()))
                .map(PartialReflect::into_partial_reflect)
                .unwrap_or(value);
            components.push(value);
        }
        Ok(components)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_object::types::{ObjectCategory, VisualKind};
    use crate::Health;

    fn test_registry() -> TypeRegistry {
        let mut registry = TypeRegistry::default();
        registry.register::<Health>();
        registry.register::<ObjectCategory>();
        registry.register::<VisualKind>();
        registry
    }

    #[test]
    fn deserialize_valid_world_object() {
        let registry = test_registry();
        let ron = br#"{
            "protocol::world_object::types::ObjectCategory": Scenery,
            "protocol::world_object::types::VisualKind": Vox("models/trees/tree_circle.vox"),
            "protocol::Health": (current: 50.0, max: 50.0),
        }"#;
        let def = deserialize_world_object(ron, &registry).unwrap();
        assert_eq!(def.components.len(), 3);
    }

    #[test]
    fn deserialize_empty_components() {
        let registry = test_registry();
        let ron = br#"{}"#;
        let def = deserialize_world_object(ron, &registry).unwrap();
        assert!(def.components.is_empty());
    }

    #[test]
    fn deserialize_unregistered_type_errors() {
        let registry = TypeRegistry::default();
        let ron = br#"{
            "protocol::Health": (current: 1.0, max: 1.0),
        }"#;
        assert!(deserialize_world_object(ron, &registry).is_err());
    }

    #[test]
    fn deserialize_malformed_ron_errors() {
        let registry = test_registry();
        assert!(deserialize_world_object(b"not valid ron {{{", &registry).is_err());
    }
}
