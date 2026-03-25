use super::loader::{apply_ability_archetype, extract_phases};
use super::loading::DefaultAbilitySlots;
use super::types::{
    AbilityAsset, AbilityCooldowns, AbilityDefs, AbilityPhase, AbilityPhases, AbilitySlots,
    ActiveAbility, OnHitEffectDefs, OnHitEffects,
};
use crate::{PlayerActions, PlayerId};
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::LocalTimeline;
use lightyear::prelude::{
    ControlledBy, NetworkTarget, PreSpawned, PredictionDespawnCommandsExt, PredictionTarget,
    Replicate, Tick,
};
use tracy_client::{plot, Client as TracyClient};

const ABILITY_ACTIONS: [PlayerActions; 4] = [
    PlayerActions::Ability1,
    PlayerActions::Ability2,
    PlayerActions::Ability3,
    PlayerActions::Ability4,
];

/// Maps a `PlayerActions` ability variant to a slot index (0-3).
pub fn ability_action_to_slot(action: &PlayerActions) -> Option<usize> {
    ABILITY_ACTIONS.iter().position(|a| a == action)
}

/// Maps a slot index (0-3) to its corresponding `PlayerActions` variant.
pub fn slot_to_ability_action(slot: usize) -> Option<PlayerActions> {
    ABILITY_ACTIONS.get(slot).copied()
}

pub fn ability_activation(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    ability_assets: Res<Assets<AbilityAsset>>,
    registry: Res<AppTypeRegistry>,
    default_slots: Res<DefaultAbilitySlots>,
    timeline: Res<LocalTimeline>,
    mut query: Query<(
        Entity,
        &ActionState<PlayerActions>,
        Option<&AbilitySlots>,
        &mut AbilityCooldowns,
        &PlayerId,
    )>,
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();

    for (entity, action_state, slots_opt, mut cooldowns, player_id) in &mut query {
        // Plot raw just_pressed state for Ability2 (speed_burst) every FixedUpdate tick
        plot!(
            "ability2_just_pressed",
            if action_state.just_pressed(&PlayerActions::Ability2) {
                1.0
            } else {
                0.0
            }
        );
        plot!(
            "ability2_pressed",
            if action_state.pressed(&PlayerActions::Ability2) {
                1.0
            } else {
                0.0
            }
        );
        let slots = slots_opt.unwrap_or(&default_slots.0);
        for (slot_idx, action) in ABILITY_ACTIONS.iter().enumerate() {
            // Use `pressed()` instead of `just_pressed()` because lightyear's
            // input snapshot deserialization loses the JustPressed transient state —
            // the server only ever sees Pressed. The cooldown check below prevents
            // re-activation while the button is held.
            if !action_state.pressed(action) {
                continue;
            }
            let Some(ref ability_id) = slots.0[slot_idx] else {
                continue;
            };
            let Some(handle) = ability_defs.get(ability_id) else {
                warn!("Ability {:?} not found in defs", ability_id);
                continue;
            };
            let Some(asset) = ability_assets.get(handle) else {
                warn!("Ability {:?} asset not loaded", ability_id);
                continue;
            };
            let Some(phases) = extract_phases(asset) else {
                warn!("Ability {:?} missing AbilityPhases component", ability_id);
                continue;
            };
            if cooldowns.is_on_cooldown(slot_idx, tick, phases.cooldown) {
                continue;
            }

            cooldowns.last_used[slot_idx] = Some(tick);
            if let Some(client) = TracyClient::running() {
                client.message(
                    &format!("ability_activated slot={} id={:?}", slot_idx, ability_id),
                    0,
                );
            }
            let salt = (player_id.0.to_bits()) << 32 | (slot_idx as u64) << 16 | 0u64;

            let entity_id = commands
                .spawn((
                    ActiveAbility {
                        def_id: ability_id.clone(),
                        caster: entity,
                        original_caster: entity,
                        target: entity,
                        phase: AbilityPhase::Startup,
                        phase_start_tick: tick,
                        ability_slot: slot_idx as u8,
                        depth: 0,
                    },
                    PreSpawned::default_with_salt(salt),
                    Name::new("ActiveAbility"),
                ))
                .id();

            apply_ability_archetype(&mut commands, entity_id, asset, registry.0.clone());

            if let Ok(controlled_by) = server_query.get(entity) {
                commands.entity(entity_id).insert((
                    Replicate::to_clients(NetworkTarget::All),
                    PredictionTarget::to_clients(NetworkTarget::All),
                    *controlled_by,
                ));
            }
        }
    }
}

fn advance_ability_phase(
    commands: &mut Commands,
    entity: Entity,
    active: &mut ActiveAbility,
    phases: &AbilityPhases,
    tick: Tick,
) {
    let elapsed = tick - active.phase_start_tick;
    let phase_complete = elapsed >= phases.phase_duration(&active.phase) as i16;

    if !phase_complete {
        return;
    }

    match active.phase {
        AbilityPhase::Startup => {
            active.phase = AbilityPhase::Active;
            active.phase_start_tick = tick;
        }
        AbilityPhase::Active => {
            active.phase = AbilityPhase::Recovery;
            active.phase_start_tick = tick;
        }
        AbilityPhase::Recovery => {
            commands.entity(entity).prediction_despawn();
        }
    }
}

pub fn update_active_abilities(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    mut query: Query<(
        Entity,
        &mut ActiveAbility,
        &AbilityPhases,
        Option<&OnHitEffectDefs>,
    )>,
) {
    let tick = timeline.tick();

    for (entity, mut active, phases, on_hit_defs) in &mut query {
        let prev_phase = active.phase.clone();
        advance_ability_phase(&mut commands, entity, &mut active, phases, tick);

        if active.phase == AbilityPhase::Active && prev_phase != AbilityPhase::Active {
            if let Some(defs) = on_hit_defs {
                if !defs.0.is_empty() {
                    commands.entity(entity).insert(OnHitEffects {
                        effects: defs.0.clone(),
                        caster: active.caster,
                        original_caster: active.original_caster,
                        depth: active.depth,
                    });
                }
            }
        }

        if active.phase != AbilityPhase::Active && prev_phase == AbilityPhase::Active {
            commands.entity(entity).remove::<OnHitEffects>();
        }
    }
}
