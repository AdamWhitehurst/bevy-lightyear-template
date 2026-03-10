pub mod asset;
pub mod spawn;

use asset::*;
use bevy::prelude::*;
use bevy_common_assets::ron::RonAssetPlugin;
use protocol::{app_state::TrackedAssets, CharacterType};
use std::collections::HashMap;

pub use spawn::{AnimSetRef, BoneEntities, Facing, RigBillboard, SpriteRig};

pub struct SpriteRigPlugin;

impl Plugin for SpriteRigPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            RonAssetPlugin::<SpriteRigAsset>::new(&["rig.ron"]),
            RonAssetPlugin::<SpriteAnimAsset>::new(&["anim.ron"]),
            RonAssetPlugin::<SpriteAnimSetAsset>::new(&["animset.ron"]),
        ));
        app.add_systems(Startup, load_rig_assets);
        app.add_systems(
            Update,
            (
                spawn::resolve_character_rig,
                spawn::spawn_sprite_rigs,
                spawn::billboard_rigs_face_camera,
                spawn::update_facing_from_velocity,
                spawn::apply_facing_to_rig,
            )
                .chain(),
        );
    }
}

/// Maps `CharacterType` to its loaded rig and animset handles.
#[derive(Resource)]
pub struct RigRegistry {
    pub entries: HashMap<CharacterType, RigRegistryEntry>,
}

/// Loaded handles for one character type's rig and animset.
pub struct RigRegistryEntry {
    pub animset_handle: Handle<SpriteAnimSetAsset>,
    pub rig_handle: Handle<SpriteRigAsset>,
}

fn load_rig_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<TrackedAssets>,
) {
    let mut entries = HashMap::new();

    let animset_handle =
        asset_server.load::<SpriteAnimSetAsset>("anims/humanoid/humanoid.animset.ron");
    let rig_handle = asset_server.load::<SpriteRigAsset>("rigs/humanoid.rig.ron");
    tracked.add(animset_handle.clone());
    tracked.add(rig_handle.clone());
    entries.insert(
        CharacterType::Humanoid,
        RigRegistryEntry {
            animset_handle,
            rig_handle,
        },
    );

    commands.insert_resource(RigRegistry { entries });
}
