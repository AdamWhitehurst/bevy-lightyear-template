pub mod panels;
pub mod state;

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use protocol::app_state::{AppState, IdentityLoadComplete, RelayPoolReady};
use protocol::billboard::billboard_material::BillboardMaterial;
use protocol::billboard::shadow_only_material::ShadowOnlyMaterial;
use protocol::billboard::sprite_rig_material::SpriteRigMaterial;
use protocol::{CharacterMarker, CharacterType};

/// Boots the live-preview editor: the real `sprite_rig` runtime (materials + spawn +
/// animation chain) minus physics/networking, one humanoid rig, and a static camera.
///
/// Registers the rig materials and `SpriteRigPlugin` directly instead of depending on
/// `render::RenderPlugin`: that plugin's camera systems take avian's `SpatialQuery`
/// (panics without `PhysicsPlugins`) and query lightyear's `Controlled`, neither of
/// which exists in the editor.
pub struct AnimationEditorPlugin;

impl Plugin for AnimationEditorPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::pbr::MaterialPlugin::<BillboardMaterial>::default());
        app.add_plugins(bevy::pbr::MaterialPlugin::<SpriteRigMaterial>::default());
        app.add_plugins(bevy::pbr::MaterialPlugin::<ShadowOnlyMaterial>::default());
        app.add_plugins(sprite_rig::SpriteRigPlugin);
        app.add_systems(Startup, (satisfy_app_state_gates, setup_editor_scene));
        app.add_systems(Update, spawn_editor_rig);
        app.add_systems(
            Update,
            (
                state::init_editor_state.run_if(
                    in_state(AppState::Ready).and(not(resource_exists::<state::EditorState>)),
                ),
                state::drive_player_from_playhead.run_if(resource_exists::<state::EditorState>),
            ),
        );
        app.add_systems(
            EguiPrimaryContextPass,
            panels::transport::draw_transport.run_if(resource_exists::<state::EditorState>),
        );
    }
}

/// Spawns a humanoid rig driven by the real spawn + animation chain
/// (`spawn_sprite_rigs` → graph build → locomotion blend), once the rig, animset, and all
/// sprite images are loaded — `spawn_sprite_rigs` panics on unloaded assets, an invariant
/// the game satisfies by replicating characters in only after `AppState::Ready`.
///
/// Resolves `SpriteRig`/`AnimSetRef`/`Facing` itself instead of going through
/// `resolve_character_rig`: that system only matches replicated entities
/// (`Predicted`/`Replicated`/`Interpolated`), which the editor rig is not.
///
/// `LinearVelocity` is the input `update_locomotion_blend_weights` reads; zero holds the
/// rig in idle until the audition controls (Phase 9) drive it. No physics body, no
/// replication — the editor rig is animation-only.
fn spawn_editor_rig(
    mut commands: Commands,
    existing: Query<(), With<CharacterMarker>>,
    registry: Res<sprite_rig::RigRegistry>,
    sprite_images: Res<sprite_rig::SpriteImageHandles>,
    asset_server: Res<AssetServer>,
) {
    if !existing.is_empty() {
        return; // rig already spawned — steady state
    }
    let entry = registry
        .entries
        .get(&CharacterType::Humanoid)
        .expect("RigRegistry missing Humanoid entry");
    if !asset_server.is_loaded_with_dependencies(&entry.rig_handle)
        || !asset_server.is_loaded_with_dependencies(&entry.animset_handle)
    {
        trace!("editor rig/animset assets still loading; retrying next frame");
        return;
    }
    let all_images_loaded = sprite_images
        .0
        .values()
        .all(|handle| asset_server.is_loaded_with_dependencies(handle));
    if sprite_images.0.is_empty() || !all_images_loaded {
        trace!("editor sprite images still loading; retrying next frame");
        return;
    }
    commands.spawn((
        CharacterMarker,
        CharacterType::Humanoid,
        sprite_rig::SpriteRig(entry.rig_handle.clone()),
        sprite_rig::AnimSetRef(entry.animset_handle.clone()),
        sprite_rig::Facing::Right,
        LinearVelocity::ZERO,
        Transform::default(),
        Visibility::default(),
    ));
}

/// The client reaches `AppState::Ready` only when `RelayPoolReady` and
/// `IdentityLoadComplete` are set (by nostr/identity systems). The editor has neither, so
/// set both at startup; `check_assets_loaded` then advances to `Ready` once the rig,
/// animset, and sprite images finish loading.
fn satisfy_app_state_gates(
    mut relay: ResMut<RelayPoolReady>,
    mut identity: ResMut<IdentityLoadComplete>,
) {
    relay.0 = true;
    identity.0 = true;
}

/// Static camera framing the rig at the origin, plus a directional light. The rig
/// billboards its joints toward the camera (`billboard_joint_roots`), so a fixed
/// camera always sees the sprite plane face-on.
fn setup_editor_scene(mut commands: Commands) {
    // The humanoid rig spans roughly y ∈ [-2.5, 0.5] around the entity origin
    // (root bone sits at y = -1.75), so the camera aims at its vertical center.
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, -0.75, -16.0).looking_at(Vec3::new(0.0, -0.75, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::default().looking_to(Vec3::new(-0.5, -1.0, -0.5), Vec3::Y),
    ));
}
