use bevy::prelude::*;
use bevy::reflect::serde::{TypeRegistrationDeserializer, TypedReflectDeserializer};
use bevy::reflect::{PartialReflect, ReflectFromReflect, TypeRegistry};
use serde::de::{DeserializeSeed, Deserializer, MapAccess, Visitor};
use std::fmt;

/// `DeserializeSeed` that reads a flat `{ "type::Path": (data) }` RON map
/// into a `Vec<Box<dyn PartialReflect>>`.
pub struct ComponentMapDeserializer<'a> {
    pub registry: &'a TypeRegistry,
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

/// Deserialize a `Vec<Box<dyn PartialReflect>>` from RON bytes using a flat
/// `{ "type::Path": (data) }` map format.
pub fn deserialize_component_map(
    bytes: &[u8],
    registry: &TypeRegistry,
) -> Result<Vec<Box<dyn PartialReflect>>, ReflectLoadError> {
    let mut deserializer = ron::de::Deserializer::from_bytes(bytes)?;
    let components = ComponentMapDeserializer { registry }.deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(components)
}

/// Error type for reflect-based asset loading failures.
#[derive(Debug)]
pub enum ReflectLoadError {
    Io(std::io::Error),
    Ron(ron::error::SpannedError),
}

impl fmt::Display for ReflectLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Ron(e) => write!(f, "RON error: {e}"),
        }
    }
}

impl std::error::Error for ReflectLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Ron(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for ReflectLoadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ron::error::SpannedError> for ReflectLoadError {
    fn from(e: ron::error::SpannedError) -> Self {
        Self::Ron(e)
    }
}

impl From<ron::error::Error> for ReflectLoadError {
    fn from(e: ron::error::Error) -> Self {
        Self::Ron(ron::error::SpannedError {
            code: e,
            span: ron::error::Span {
                start: ron::error::Position { line: 0, col: 0 },
                end: ron::error::Position { line: 0, col: 0 },
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ability::{
        AbilityEffect, AbilityPhases, EffectTarget, ForceFrame, InputEffect, OnHitEffectDefs,
        OnInputEffects, OnTickEffects, TickEffect, WhileActiveEffects,
    };
    use crate::NetworkedPlayerActions;
    use bevy::reflect::TypeRegistry;

    fn ability_test_registry() -> TypeRegistry {
        let mut registry = TypeRegistry::default();
        registry.register::<AbilityPhases>();
        registry.register::<OnTickEffects>();
        registry.register::<TickEffect>();
        registry.register::<WhileActiveEffects>();
        registry.register::<OnHitEffectDefs>();
        registry.register::<OnInputEffects>();
        registry.register::<InputEffect>();
        registry.register::<AbilityEffect>();
        registry.register::<EffectTarget>();
        registry.register::<ForceFrame>();
        registry.register::<NetworkedPlayerActions>();
        registry
    }

    #[test]
    fn deserialize_ability_phases() {
        let registry = ability_test_registry();
        let ron = br#"{
            "protocol::ability::AbilityPhases": (startup: 4, active: 20, recovery: 0, cooldown: 16),
        }"#;
        let components = deserialize_component_map(ron, &registry).unwrap();
        assert_eq!(components.len(), 1);

        let phases = components[0]
            .try_downcast_ref::<AbilityPhases>()
            .expect("should downcast to AbilityPhases");
        assert_eq!(phases.startup, 4);
        assert_eq!(phases.active, 20);
        assert_eq!(phases.recovery, 0);
        assert_eq!(phases.cooldown, 16);
    }

    #[test]
    fn deserialize_ability_with_multiple_components() {
        let registry = ability_test_registry();
        let ron = br#"#![enable(implicit_some)]
        {
            "protocol::ability::AbilityPhases": (startup: 4, active: 20, recovery: 0, cooldown: 16),
            "protocol::ability::OnTickEffects": ([(tick: 0, effect: Melee())]),
            "protocol::ability::OnHitEffectDefs": ([
                Damage(amount: 5.0, target: Victim),
                ApplyForce(force: (0.0, 0.9, 2.85), frame: RelativePosition, target: Victim),
            ]),
            "protocol::ability::OnInputEffects": ([(action: Ability1, effect: Ability(id: "punch2", target: Caster))]),
        }"#;
        let components = deserialize_component_map(ron, &registry).unwrap();
        assert_eq!(components.len(), 4);
    }

    #[test]
    fn deserialize_newtype_tuple_struct_syntax() {
        let registry = ability_test_registry();
        let ron = br#"{
            "protocol::ability::WhileActiveEffects": ([
                SetVelocity(speed: 15.0, target: Caster),
            ]),
        }"#;
        let components = deserialize_component_map(ron, &registry).unwrap();
        assert_eq!(components.len(), 1);

        let effects = components[0]
            .try_downcast_ref::<WhileActiveEffects>()
            .expect("should downcast to WhileActiveEffects");
        assert_eq!(effects.0.len(), 1);
    }
}
