#import bevy_pbr::forward_io::VertexOutput

@fragment
fn fragment(_in: VertexOutput) -> @location(0) vec4<f32> {
    discard;
    return vec4<f32>(0.0);
}
