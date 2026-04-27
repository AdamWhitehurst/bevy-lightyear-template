use super::activation::{ability_activation, update_active_abilities};
use super::effects::{
    apply_on_end_effects, apply_on_input_effects, apply_on_tick_effects, apply_while_active_effects,
};
use super::lifecycle::{
    ability_bullet_lifetime, aoe_hitbox_lifetime, despawn_active_ability_on_removal, expire_buffs,
};
use super::loader::AbilityAssetLoader;
use super::loading::{
    insert_ability_defs, load_ability_defs, load_default_ability_slots, reload_ability_defs,
    sync_default_ability_slots,
};
use super::spawn::{
    ability_projectile_spawn, despawn_ability_projectile_spawn, handle_ability_projectile_spawn,
};
use super::types::AbilityDefs;
use super::types::{
    AbilityAsset, AbilityEffect, AbilityPhases, AbilitySlots, Condition, ConditionalEffect,
    ConditionalEffects, EffectTarget, ForceFrame, InputEffect, OnEndEffects, OnHitEffectDefs,
    OnInputEffects, OnTickEffects, TickEffect, WhileActiveEffects,
};
use crate::PlayerActions;
use bevy::prelude::*;

#[cfg(target_arch = "wasm32")]
use super::loading::trigger_individual_ability_loads;
#[cfg(target_arch = "wasm32")]
use super::types::AbilityManifest;

pub struct AbilityPlugin;

impl Plugin for AbilityPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<AbilityPhases>()
            .register_type::<OnTickEffects>()
            .register_type::<TickEffect>()
            .register_type::<WhileActiveEffects>()
            .register_type::<OnHitEffectDefs>()
            .register_type::<OnEndEffects>()
            .register_type::<OnInputEffects>()
            .register_type::<InputEffect>()
            .register_type::<AbilityEffect>()
            .register_type::<EffectTarget>()
            .register_type::<ForceFrame>()
            .register_type::<PlayerActions>()
            .register_type::<Condition>()
            .register_type::<ConditionalEffect>()
            .register_type::<ConditionalEffects>();

        app.init_asset::<AbilityAsset>()
            .init_asset_loader::<AbilityAssetLoader>();
        app.add_plugins(
            bevy_common_assets::ron::RonAssetPlugin::<AbilitySlots>::new(&["ability_slots.ron"]),
        );

        #[cfg(target_arch = "wasm32")]
        app.add_plugins(
            bevy_common_assets::ron::RonAssetPlugin::<AbilityManifest>::new(&[
                "abilities.manifest.ron",
            ]),
        );

        app.add_systems(Startup, (load_ability_defs, load_default_ability_slots));

        #[cfg(target_arch = "wasm32")]
        app.add_systems(
            PreUpdate,
            trigger_individual_ability_loads.run_if(in_state(crate::app_state::AppState::Loading)),
        );

        app.add_systems(
            Update,
            (
                insert_ability_defs.run_if(not(resource_exists::<AbilityDefs>)),
                reload_ability_defs,
                sync_default_ability_slots,
            ),
        );

        app.add_message::<crate::DeathEvent>();

        let ready = in_state(crate::app_state::AppState::Ready);

        app.add_systems(
            FixedUpdate,
            (
                ability_activation,
                update_active_abilities,
                apply_on_tick_effects,
                apply_while_active_effects,
                apply_on_end_effects,
                apply_on_input_effects,
                ability_projectile_spawn,
            )
                .chain()
                .run_if(ready.clone()),
        );

        app.add_systems(
            FixedUpdate,
            (
                crate::hit_detection::update_hitbox_positions,
                crate::hit_detection::process_hitbox_hits,
                crate::hit_detection::process_projectile_hits,
                crate::hit_detection::cleanup_hitbox_entities,
            )
                .chain()
                .after(apply_on_tick_effects)
                .run_if(ready.clone()),
        );

        app.add_systems(
            FixedUpdate,
            (expire_buffs, aoe_hitbox_lifetime, ability_bullet_lifetime)
                .after(crate::hit_detection::process_hitbox_hits)
                .after(crate::hit_detection::process_projectile_hits)
                .run_if(ready.clone()),
        );
        app.add_systems(PreUpdate, handle_ability_projectile_spawn);
        app.add_observer(despawn_ability_projectile_spawn);
        app.add_observer(despawn_active_ability_on_removal);
    }
}
