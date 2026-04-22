#import bevy_pbr::{
    mesh_functions,
    skinning,
    forward_io::{Vertex, VertexOutput},
    mesh_view_bindings::view,
}

@vertex
fn vertex(vertex_no_morph: Vertex) -> VertexOutput {
    var out: VertexOutput;
    var vertex = vertex_no_morph;

#ifdef SKINNED
    var world_from_local = skinning::skin_model(
        vertex.joint_indices,
        vertex.joint_weights,
        vertex_no_morph.instance_index,
    );
#else
    var world_from_local = mesh_functions::get_world_from_local(
        vertex_no_morph.instance_index,
    );
#endif

    // Sprite-rig billboard with Z-rotation preservation.
    //
    // The joint hierarchy contains a Y-rotation (from billboard_joint_roots)
    // that positions bones in the camera-facing plane, plus per-bone
    // Z-rotations from animation. We need to:
    //   1. Strip all rotation (making quads face the screen)
    //   2. Re-apply ONLY the bone Z-rotation as screen-plane tilt
    //
    // For combined Ry(θy) * Rz(θz):
    //   col0 = (cos θy · cos θz,  sin θz,  -sin θy · cos θz)
    //   col1 = (-cos θy · sin θz, cos θz,   sin θy · sin θz)
    //
    // col0.y is sin(θz) signed; col1.y is cos(θz) signed — both immune to θy.
    // Using col1.y (rather than length(col0.xz)) recovers the full ±π range,
    // so bones can rotate fully (e.g. arms raised overhead) without aliasing.

    var model_view = view.view_from_world * world_from_local;

    let scale_x = length(model_view[0].xyz);
    let scale_y = length(model_view[1].xyz);
    let scale_z = length(model_view[2].xyz);

    let det = determinant(mat3x3f(
        model_view[0].xyz,
        model_view[1].xyz,
        model_view[2].xyz,
    ));
    let flip = select(1.0, -1.0, det < 0.0);

    let raw_sin = world_from_local[0].y;
    let raw_cos = world_from_local[1].y;
    let rot_len = sqrt(raw_cos * raw_cos + raw_sin * raw_sin);
    let sin_t = select(0.0, raw_sin / rot_len, rot_len > 0.0001);
    let cos_t = select(1.0, raw_cos / rot_len, rot_len > 0.0001);

    // Spherical billboard + screen-plane bone Z-rotation.
    // flip mirrors the X axis only: column 0.x and column 1.x carry the flip,
    // while .y components remain unflipped so the rotation visually mirrors.
    model_view[0] = vec4<f32>(flip * scale_x * cos_t, scale_x * sin_t, 0.0, model_view[0][3]);
    model_view[1] = vec4<f32>(-flip * scale_y * sin_t, scale_y * cos_t, 0.0, model_view[1][3]);
    model_view[2] = vec4<f32>(0.0, 0.0, scale_z, model_view[2][3]);

    let view_pos = model_view * vec4<f32>(vertex.position, 1.0);
    out.position = view.clip_from_view * view_pos;

    let world_pos = world_from_local * vec4<f32>(vertex.position, 1.0);
    out.world_position = world_pos;

#ifdef VERTEX_NORMALS
    out.world_normal = normalize(
        (view.world_from_view * vec4<f32>(0.0, 0.0, 1.0, 0.0)).xyz
    );
#endif

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif

#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex_no_morph.instance_index;
#endif

    return out;
}
