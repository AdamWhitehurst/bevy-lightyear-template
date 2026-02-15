use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use lightyear::core::time::TickDelta;
use lightyear::prelude::{LocalTimeline, NetworkTimeline, Tick};
use protocol::*;
use std::collections::HashMap;

fn test_defs() -> HashMap<AbilityId, AbilityDef> {
    let mut m = HashMap::new();
    m.insert(
        AbilityId("punch".into()),
        AbilityDef {
            startup_ticks: 4,
            active_ticks: 3,
            recovery_ticks: 6,
            cooldown_ticks: 16,
            steps: 3,
            step_window_ticks: 20,
            effect: AbilityEffect::Melee {
                knockback_force: 15.0,
            },
        },
    );
    m.insert(
        AbilityId("dash".into()),
        AbilityDef {
            startup_ticks: 2,
            active_ticks: 8,
            recovery_ticks: 4,
            cooldown_ticks: 64,
            steps: 1,
            step_window_ticks: 0,
            effect: AbilityEffect::Dash { speed: 15.0 },
        },
    );
    m.insert(
        AbilityId("fireball".into()),
        AbilityDef {
            startup_ticks: 6,
            active_ticks: 2,
            recovery_ticks: 8,
            cooldown_ticks: 96,
            steps: 1,
            step_window_ticks: 0,
            effect: AbilityEffect::Projectile {
                speed: 20.0,
                lifetime_ticks: 192,
                knockback_force: 20.0,
            },
        },
    );
    m
}

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(AbilityDefs {
        abilities: test_defs(),
    });
    app.add_systems(
        Update,
        (
            ability::ability_activation,
            ability::update_active_abilities,
            ability::dispatch_effect_markers,
            ability::ability_dash_effect,
        )
            .chain(),
    );
    app.add_systems(Update, ability::ability_bullet_lifetime);
    app
}

fn spawn_timeline(world: &mut World, tick_value: u16) -> Entity {
    let entity = world.spawn(LocalTimeline::default()).id();
    let mut timeline = world.get_mut::<LocalTimeline>(entity).unwrap();
    timeline.apply_delta(TickDelta::from_i16(tick_value as i16));
    entity
}

fn advance_timeline(world: &mut World, timeline_entity: Entity, delta: i16) {
    let mut timeline = world.get_mut::<LocalTimeline>(timeline_entity).unwrap();
    timeline.apply_delta(TickDelta::from_i16(delta));
}

fn punch_slots() -> AbilitySlots {
    AbilitySlots([
        Some(AbilityId("punch".into())),
        Some(AbilityId("dash".into())),
        Some(AbilityId("fireball".into())),
        None,
    ])
}

fn spawn_character(world: &mut World) -> Entity {
    world
        .spawn((
            CharacterMarker,
            ActionState::<PlayerActions>::default(),
            punch_slots(),
            AbilityCooldowns::default(),
            avian3d::prelude::Position(Vec3::ZERO),
            avian3d::prelude::Rotation::default(),
            avian3d::prelude::LinearVelocity(Vec3::ZERO),
        ))
        .id()
}

#[test]
fn activation_on_press() {
    let mut app = test_app();
    let timeline_entity = spawn_timeline(app.world_mut(), 100);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .get_mut::<ActionState<PlayerActions>>(char_entity)
        .unwrap()
        .press(&PlayerActions::Ability1);

    app.update();

    let active = app.world().get::<ActiveAbility>(char_entity);
    assert!(active.is_some(), "ActiveAbility should be inserted");
    let active = active.unwrap();
    assert_eq!(active.ability_id, AbilityId("punch".into()));
    assert_eq!(active.step, 0);
    assert_eq!(active.total_steps, 3);

    let timeline = app.world().get::<LocalTimeline>(timeline_entity).unwrap();
    assert_eq!(active.phase_start_tick, timeline.tick());
}

#[test]
fn activation_blocked_by_cooldown() {
    let mut app = test_app();
    spawn_timeline(app.world_mut(), 100);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .get_mut::<AbilityCooldowns>(char_entity)
        .unwrap()
        .last_used[0] = Some(Tick(90));

    app.world_mut()
        .get_mut::<ActionState<PlayerActions>>(char_entity)
        .unwrap()
        .press(&PlayerActions::Ability1);

    app.update();

    assert!(
        app.world().get::<ActiveAbility>(char_entity).is_none(),
        "Should not activate while on cooldown"
    );
}

#[test]
fn activation_empty_slot() {
    let mut app = test_app();
    spawn_timeline(app.world_mut(), 100);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .get_mut::<ActionState<PlayerActions>>(char_entity)
        .unwrap()
        .press(&PlayerActions::Ability4);

    app.update();

    assert!(
        app.world().get::<ActiveAbility>(char_entity).is_none(),
        "Should not activate empty slot"
    );
}

#[test]
fn activation_sets_cooldown() {
    let mut app = test_app();
    let timeline_entity = spawn_timeline(app.world_mut(), 100);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .get_mut::<ActionState<PlayerActions>>(char_entity)
        .unwrap()
        .press(&PlayerActions::Ability1);

    app.update();

    let cd = app.world().get::<AbilityCooldowns>(char_entity).unwrap();
    let timeline = app.world().get::<LocalTimeline>(timeline_entity).unwrap();
    assert_eq!(cd.last_used[0], Some(timeline.tick()));
}

#[test]
fn activation_blocked_by_active() {
    let mut app = test_app();
    spawn_timeline(app.world_mut(), 100);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .entity_mut(char_entity)
        .insert(ActiveAbility {
            ability_id: AbilityId("punch".into()),
            phase: AbilityPhase::Active,
            phase_start_tick: Tick(95),
            step: 0,
            total_steps: 3,
            chain_input_received: false,
        });

    app.world_mut()
        .get_mut::<ActionState<PlayerActions>>(char_entity)
        .unwrap()
        .press(&PlayerActions::Ability2);

    app.update();

    let active = app.world().get::<ActiveAbility>(char_entity).unwrap();
    assert_eq!(active.ability_id, AbilityId("punch".into()));
}

#[test]
fn phase_startup_to_active() {
    let mut app = test_app();
    let timeline_entity = spawn_timeline(app.world_mut(), 100);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .entity_mut(char_entity)
        .insert(ActiveAbility {
            ability_id: AbilityId("punch".into()),
            phase: AbilityPhase::Startup,
            phase_start_tick: Tick(100),
            step: 0,
            total_steps: 3,
            chain_input_received: false,
        });

    advance_timeline(app.world_mut(), timeline_entity, 4);
    app.update();

    let active = app.world().get::<ActiveAbility>(char_entity).unwrap();
    assert_eq!(active.phase, AbilityPhase::Active);
}

#[test]
fn phase_active_to_recovery() {
    let mut app = test_app();
    let timeline_entity = spawn_timeline(app.world_mut(), 200);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .entity_mut(char_entity)
        .insert(ActiveAbility {
            ability_id: AbilityId("punch".into()),
            phase: AbilityPhase::Active,
            phase_start_tick: Tick(200),
            step: 0,
            total_steps: 3,
            chain_input_received: false,
        });

    advance_timeline(app.world_mut(), timeline_entity, 3);
    app.update();

    let active = app.world().get::<ActiveAbility>(char_entity).unwrap();
    assert_eq!(active.phase, AbilityPhase::Recovery);
}

#[test]
fn phase_recovery_completes_single_step() {
    let mut app = test_app();
    let timeline_entity = spawn_timeline(app.world_mut(), 300);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .entity_mut(char_entity)
        .insert(ActiveAbility {
            ability_id: AbilityId("dash".into()),
            phase: AbilityPhase::Recovery,
            phase_start_tick: Tick(300),
            step: 0,
            total_steps: 1,
            chain_input_received: false,
        });

    advance_timeline(app.world_mut(), timeline_entity, 4);
    app.update();

    assert!(
        app.world().get::<ActiveAbility>(char_entity).is_none(),
        "ActiveAbility should be removed after recovery completes"
    );
}

#[test]
fn combo_chain_advances_step() {
    let mut app = test_app();
    let timeline_entity = spawn_timeline(app.world_mut(), 400);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .entity_mut(char_entity)
        .insert(ActiveAbility {
            ability_id: AbilityId("punch".into()),
            phase: AbilityPhase::Recovery,
            phase_start_tick: Tick(400),
            step: 0,
            total_steps: 3,
            chain_input_received: true,
        });

    advance_timeline(app.world_mut(), timeline_entity, 6);
    app.update();

    let active = app.world().get::<ActiveAbility>(char_entity).unwrap();
    assert_eq!(active.step, 1, "Step should have advanced");
    assert_eq!(
        active.phase,
        AbilityPhase::Startup,
        "Should restart at Startup"
    );
    assert!(!active.chain_input_received, "chain_input should be reset");
}

#[test]
fn combo_window_expires() {
    let mut app = test_app();
    let timeline_entity = spawn_timeline(app.world_mut(), 500);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .entity_mut(char_entity)
        .insert(ActiveAbility {
            ability_id: AbilityId("punch".into()),
            phase: AbilityPhase::Recovery,
            phase_start_tick: Tick(500),
            step: 0,
            total_steps: 3,
            chain_input_received: false,
        });

    advance_timeline(app.world_mut(), timeline_entity, 20);
    app.update();

    assert!(
        app.world().get::<ActiveAbility>(char_entity).is_none(),
        "ActiveAbility should be removed when chain window expires"
    );
}

#[test]
fn dash_applies_velocity_active() {
    let mut app = test_app();
    spawn_timeline(app.world_mut(), 100);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .entity_mut(char_entity)
        .insert(ActiveAbility {
            ability_id: AbilityId("dash".into()),
            phase: AbilityPhase::Active,
            phase_start_tick: Tick(100),
            step: 0,
            total_steps: 1,
            chain_input_received: false,
        });

    app.update();

    let vel = app
        .world()
        .get::<avian3d::prelude::LinearVelocity>(char_entity)
        .unwrap();
    assert!(
        vel.z.abs() > 10.0,
        "Dash should apply significant Z velocity, got {}",
        vel.z
    );
}

#[test]
fn dash_no_velocity_startup() {
    let mut app = test_app();
    spawn_timeline(app.world_mut(), 100);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .entity_mut(char_entity)
        .insert(ActiveAbility {
            ability_id: AbilityId("dash".into()),
            phase: AbilityPhase::Startup,
            phase_start_tick: Tick(100),
            step: 0,
            total_steps: 1,
            chain_input_received: false,
        });

    app.update();

    let vel = app
        .world()
        .get::<avian3d::prelude::LinearVelocity>(char_entity)
        .unwrap();
    assert_eq!(vel.0, Vec3::ZERO, "No velocity during Startup phase");
}

#[test]
fn non_dash_no_velocity_change() {
    let mut app = test_app();
    spawn_timeline(app.world_mut(), 100);
    let char_entity = spawn_character(app.world_mut());

    app.world_mut()
        .entity_mut(char_entity)
        .insert(ActiveAbility {
            ability_id: AbilityId("punch".into()),
            phase: AbilityPhase::Active,
            phase_start_tick: Tick(100),
            step: 0,
            total_steps: 3,
            chain_input_received: false,
        });

    app.update();

    let vel = app
        .world()
        .get::<avian3d::prelude::LinearVelocity>(char_entity)
        .unwrap();
    assert_eq!(vel.0, Vec3::ZERO, "Melee should not change velocity");
}

#[test]
fn bullet_lifetime_despawn() {
    let mut app = test_app();
    spawn_timeline(app.world_mut(), 300);

    let spawn_entity = app
        .world_mut()
        .spawn(AbilityProjectileSpawn {
            spawn_tick: Tick(100),
            position: Vec3::ZERO,
            direction: Vec3::NEG_Z,
            speed: 20.0,
            lifetime_ticks: 192,
            knockback_force: 20.0,
            ability_id: AbilityId("fireball".into()),
            shooter: Entity::PLACEHOLDER,
        })
        .id();

    let bullet_entity = app.world_mut().spawn(AbilityBulletOf(spawn_entity)).id();

    app.update();

    assert!(
        app.world().get_entity(bullet_entity).is_err(),
        "Bullet should be despawned after lifetime expires"
    );
}

#[test]
fn bullet_lifetime_alive() {
    let mut app = test_app();
    spawn_timeline(app.world_mut(), 200);

    let spawn_entity = app
        .world_mut()
        .spawn(AbilityProjectileSpawn {
            spawn_tick: Tick(100),
            position: Vec3::ZERO,
            direction: Vec3::NEG_Z,
            speed: 20.0,
            lifetime_ticks: 192,
            knockback_force: 20.0,
            ability_id: AbilityId("fireball".into()),
            shooter: Entity::PLACEHOLDER,
        })
        .id();

    let bullet_entity = app.world_mut().spawn(AbilityBulletOf(spawn_entity)).id();

    app.update();

    assert!(
        app.world().get_entity(bullet_entity).is_ok(),
        "Bullet should survive before lifetime expires"
    );
}
