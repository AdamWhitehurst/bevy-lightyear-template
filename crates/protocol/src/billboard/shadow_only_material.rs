use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

/// Material for entities that must cast alpha-masked shadows but contribute
/// nothing to the main render pass.
///
/// The main fragment shader discards every fragment, so the mesh is invisible
/// to the camera. The prepass and shadow passes fall through to
/// `StandardMaterial`'s defaults, which honor `AlphaMode::Mask` and write
/// alpha-tested depth into the shadow map.
///
/// Works around a Bevy 0.18 limitation where `queue_shadows` only queues
/// shadow casters whose `RenderLayers` intersect the camera's, so a
/// "shadow-only layer" pattern is not viable — this material lets shadow
/// casters live on the same layer as the camera while staying visually hidden.
pub type ShadowOnlyMaterial = ExtendedMaterial<StandardMaterial, ShadowOnlyExt>;

/// Marker extension for the main-pass-discard fragment shader.
#[derive(AsBindGroup, Asset, TypePath, Clone, Default)]
pub struct ShadowOnlyExt {}

impl MaterialExtension for ShadowOnlyExt {
    fn fragment_shader() -> ShaderRef {
        "shaders/shadow_only.wgsl".into()
    }
}
