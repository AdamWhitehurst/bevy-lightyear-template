use avian3d::prelude::ColliderConstructor;
use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::reflect::serde::{TypeRegistrationDeserializer, TypedReflectDeserializer};
use bevy::reflect::{PartialReflect, ReflectFromReflect, TypePath, TypeRegistry, TypeRegistryArc};
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, Visitor};
use std::fmt;

use super::types::{ObjectCategory, VisualKind, WorldObjectDef, WorldObjectLoadError};

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
/// Pure function — usable in unit tests without a Bevy `App`.
pub fn deserialize_world_object(
    bytes: &[u8],
    registry: &TypeRegistry,
) -> Result<WorldObjectDef, WorldObjectLoadError> {
    let mut deserializer = ron::de::Deserializer::from_bytes(bytes)?;
    let def = WorldObjectDefSeed { registry }.deserialize(&mut deserializer)?;
    // `end()` returns `ron::error::Error`; `From<ron::error::Error>` handles the conversion.
    deserializer.end()?;
    Ok(def)
}

struct WorldObjectDefSeed<'a> {
    registry: &'a TypeRegistry,
}

impl<'a, 'de> DeserializeSeed<'de> for WorldObjectDefSeed<'a> {
    type Value = WorldObjectDef;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        // `deserialize_any` accepts both RON struct syntax `( key: value )` and
        // map syntax `{ key: value }`. `deserialize_map` only accepts `{ ... }`.
        deserializer.deserialize_any(WorldObjectDefVisitor {
            registry: self.registry,
        })
    }
}

struct WorldObjectDefVisitor<'a> {
    registry: &'a TypeRegistry,
}

impl<'a, 'de> Visitor<'de> for WorldObjectDefVisitor<'a> {
    type Value = WorldObjectDef;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a WorldObjectDef struct")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut category = None;
        let mut visual = None;
        let mut collider = None;
        let mut components = None;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "category" => category = Some(map.next_value::<ObjectCategory>()?),
                "visual" => visual = Some(map.next_value::<VisualKind>()?),
                "collider" => collider = Some(map.next_value::<Option<ColliderConstructor>>()?),
                "components" => {
                    components = Some(map.next_value_seed(ComponentMapDeserializer {
                        registry: self.registry,
                    })?)
                }
                other => {
                    return Err(de::Error::unknown_field(
                        other,
                        &["category", "visual", "collider", "components"],
                    ))
                }
            }
        }

        Ok(WorldObjectDef {
            category: category.ok_or_else(|| de::Error::missing_field("category"))?,
            visual: visual.ok_or_else(|| de::Error::missing_field("visual"))?,
            collider: collider.unwrap_or(None),
            components: components.unwrap_or_default(),
        })
    }
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
    use crate::Health;

    fn test_registry() -> TypeRegistry {
        let mut registry = TypeRegistry::default();
        registry.register::<Health>();
        registry
    }

    #[test]
    fn deserialize_valid_world_object() {
        let registry = test_registry();
        let ron = br#"(
            category: Scenery,
            visual: Vox("models/trees/tree_circle.vox"),
            collider: Some(Cylinder(radius: 0.5, height: 3.0)),
            components: {
                "protocol::Health": (current: 50.0, max: 50.0),
            },
        )"#;
        let def = deserialize_world_object(ron, &registry).unwrap();
        assert!(matches!(def.category, ObjectCategory::Scenery));
        assert!(matches!(def.visual, VisualKind::Vox(_)));
        assert!(def.collider.is_some());
        assert_eq!(def.components.len(), 1);
    }

    #[test]
    fn deserialize_empty_components() {
        let registry = test_registry();
        let ron = br#"(
            category: Interactive,
            visual: None,
            collider: None,
            components: {},
        )"#;
        let def = deserialize_world_object(ron, &registry).unwrap();
        assert!(def.components.is_empty());
        assert!(def.collider.is_none());
    }

    #[test]
    fn deserialize_unregistered_type_errors() {
        let registry = TypeRegistry::default();
        let ron = br#"(
            category: Scenery,
            visual: None,
            collider: None,
            components: {
                "protocol::Health": (current: 1.0, max: 1.0),
            },
        )"#;
        assert!(deserialize_world_object(ron, &registry).is_err());
    }

    #[test]
    fn deserialize_malformed_ron_errors() {
        let registry = test_registry();
        assert!(deserialize_world_object(b"not valid ron {{{", &registry).is_err());
    }

    #[test]
    fn deserialize_missing_required_field_errors() {
        let registry = test_registry();
        // Missing both `category` and `visual` — visitor will return missing_field error.
        let ron = br#"(
            collider: None,
            components: {},
        )"#;
        assert!(deserialize_world_object(ron, &registry).is_err());
    }

    #[test]
    fn deserialize_optional_fields_default() {
        let registry = test_registry();
        // `collider` and `components` are optional — omitting them should succeed.
        let ron = br#"(
            category: Scenery,
            visual: None,
        )"#;
        let result = deserialize_world_object(ron, &registry);
        // collider defaults to None, components defaults to empty vec.
        let def = result.unwrap();
        assert!(def.collider.is_none());
        assert!(def.components.is_empty());
    }
}
