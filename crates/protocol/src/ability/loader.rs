use super::types::{AbilityAsset, AbilityPhases, ConditionalEffects, OnTickEffects};
use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::reflect::{PartialReflect, TypeRegistryArc};

/// Extract AbilityPhases from an AbilityAsset's reflected components.
pub fn extract_phases(asset: &AbilityAsset) -> Option<&AbilityPhases> {
    let target_id = std::any::TypeId::of::<AbilityPhases>();
    for reflected in &asset.components {
        let info = reflected
            .get_represented_type_info()
            .expect("AbilityAsset should have type info");

        if info.type_id() == target_id {
            return reflected.try_downcast_ref::<AbilityPhases>();
        }
    }
    None
}

/// Extract `ConditionalEffects` from an `AbilityAsset`'s reflected components.
pub fn extract_conditional_effects(asset: &AbilityAsset) -> Option<&ConditionalEffects> {
    let target_id = std::any::TypeId::of::<ConditionalEffects>();
    for reflected in &asset.components {
        let info = reflected
            .get_represented_type_info()
            .expect("AbilityAsset should have type info");
        if info.type_id() == target_id {
            return reflected.try_downcast_ref::<ConditionalEffects>();
        }
    }
    None
}

/// Extract `OnTickEffects` from an `AbilityAsset`'s reflected components.
pub fn extract_on_tick_effects(asset: &AbilityAsset) -> Option<&OnTickEffects> {
    let target_id = std::any::TypeId::of::<OnTickEffects>();
    for reflected in &asset.components {
        let info = reflected
            .get_represented_type_info()
            .expect("AbilityAsset should have type info");
        if info.type_id() == target_id {
            return reflected.try_downcast_ref::<OnTickEffects>();
        }
    }
    None
}

/// Insert all reflected components from an `AbilityAsset` onto an entity.
pub(crate) fn apply_ability_archetype(
    commands: &mut Commands,
    entity: Entity,
    asset: &AbilityAsset,
    registry: TypeRegistryArc,
) {
    let components: Vec<Box<dyn PartialReflect>> = asset
        .components
        .iter()
        .map(|c| {
            c.reflect_clone()
                .expect("ability component must be cloneable")
                .into_partial_reflect()
        })
        .collect();

    commands.queue(move |world: &mut World| {
        let registry = registry.read();
        let mut entity_mut = world.entity_mut(entity);
        for component in &components {
            let type_path = component.reflect_type_path();
            let Some(registration) = registry.get_with_type_path(type_path) else {
                warn!("Ability component type not registered: {type_path}");
                continue;
            };
            let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                warn!("Type missing #[reflect(Component)]: {type_path}");
                continue;
            };
            reflect_component.insert(&mut entity_mut, component.as_ref(), &registry);
        }
    });
}

/// Like `apply_ability_archetype`, but replaces the asset's `OnTickEffects`
/// component with `override_on_tick` during insertion. If the asset had no
/// `OnTickEffects`, `override_on_tick` is inserted as a new component.
pub(crate) fn apply_ability_archetype_with_on_tick_override(
    commands: &mut Commands,
    entity: Entity,
    asset: &AbilityAsset,
    registry: TypeRegistryArc,
    override_on_tick: OnTickEffects,
) {
    let target_id = std::any::TypeId::of::<OnTickEffects>();
    let mut components: Vec<Box<dyn PartialReflect>> = asset
        .components
        .iter()
        .filter(|c| {
            c.get_represented_type_info()
                .map(|i| i.type_id() != target_id)
                .unwrap_or(true)
        })
        .map(|c| {
            c.reflect_clone()
                .expect("ability component must be cloneable")
                .into_partial_reflect()
        })
        .collect();
    components.push(Box::new(override_on_tick).into_partial_reflect());

    commands.queue(move |world: &mut World| {
        let registry = registry.read();
        let mut entity_mut = world.entity_mut(entity);
        for component in &components {
            let type_path = component.reflect_type_path();
            let Some(registration) = registry.get_with_type_path(type_path) else {
                warn!("Ability component type not registered: {type_path}");
                continue;
            };
            let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                warn!("Type missing #[reflect(Component)]: {type_path}");
                continue;
            };
            reflect_component.insert(&mut entity_mut, component.as_ref(), &registry);
        }
    });
}

/// Custom asset loader for `.ability.ron` files using reflect-based deserialization.
#[derive(TypePath)]
pub(super) struct AbilityAssetLoader {
    type_registry: TypeRegistryArc,
}

impl FromWorld for AbilityAssetLoader {
    fn from_world(world: &mut World) -> Self {
        Self {
            type_registry: world.resource::<AppTypeRegistry>().0.clone(),
        }
    }
}

impl AssetLoader for AbilityAssetLoader {
    type Asset = AbilityAsset;
    type Settings = ();
    type Error = crate::reflect_loader::ReflectLoadError;

    fn extensions(&self) -> &[&str] {
        &["ability.ron"]
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
        let components = crate::reflect_loader::deserialize_component_map(&bytes, &registry)?;
        Ok(AbilityAsset { components })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ability::types::{
        AbilityAsset, AbilityEffect, Condition, ConditionalEffect, ConditionalEffects,
        EffectTarget, ForceFrame,
    };
    use bevy::math::Vec3;

    fn synth_asset(ce: ConditionalEffects) -> AbilityAsset {
        AbilityAsset {
            components: vec![Box::new(ce).into_partial_reflect()],
        }
    }

    #[test]
    fn extracts_conditional_effects() {
        let ce = ConditionalEffects(vec![ConditionalEffect {
            condition: Condition::Grounded,
            effect: AbilityEffect::ApplyForce {
                force: Vec3::Y * 2000.0,
                frame: ForceFrame::World,
                target: EffectTarget::Caster,
            },
        }]);
        let asset = synth_asset(ce.clone());
        let extracted = extract_conditional_effects(&asset).unwrap();
        assert_eq!(extracted, &ce);
    }

    #[test]
    fn grounded_filter_selects_matching_entries() {
        let ce = ConditionalEffects(vec![
            ConditionalEffect {
                condition: Condition::Grounded,
                effect: AbilityEffect::ApplyForce {
                    force: Vec3::Y,
                    frame: ForceFrame::World,
                    target: EffectTarget::Caster,
                },
            },
            ConditionalEffect {
                condition: Condition::Airborne,
                effect: AbilityEffect::ApplyForce {
                    force: -Vec3::Y,
                    frame: ForceFrame::World,
                    target: EffectTarget::Caster,
                },
            },
        ]);
        let pick = |grounded: bool| -> Vec<AbilityEffect> {
            ce.0.iter()
                .filter(|c| match c.condition {
                    Condition::Grounded => grounded,
                    Condition::Airborne => !grounded,
                })
                .map(|c| c.effect.clone())
                .collect()
        };
        assert_eq!(pick(true).len(), 1);
        assert_eq!(pick(false).len(), 1);
        assert_ne!(pick(true), pick(false));
    }
}
