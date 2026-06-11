use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use protocol::app_state::{IdentityLoadComplete, RelayPoolReady};
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
        app.add_systems(
            Startup,
            (
                spawn_editor_rig,
                satisfy_app_state_gates,
                setup_editor_scene,
            ),
        );
    }
}

/// Spawns a humanoid rig driven by the real spawn + animation chain
/// (`resolve_character_rig` → `spawn_sprite_rigs` → graph build → locomotion blend).
/// `LinearVelocity` is the input `update_locomotion_blend_weights` reads; zero holds the
/// rig in idle until the audition controls (Phase 9) drive it. No physics body, no
/// replication — the editor rig is animation-only.
fn spawn_editor_rig(mut commands: Commands) {
    commands.spawn((
        CharacterMarker,
        CharacterType::Humanoid,
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
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 1.5, -6.0).looking_at(Vec3::new(0.0, 1.5, 0.0), Vec3::Y),
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
