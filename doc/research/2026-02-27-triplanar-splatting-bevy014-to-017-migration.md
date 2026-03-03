---
date: 2026-02-27T22:37:08-08:00
researcher: claude
git_commit: da4b04bc61eab50383fb714815905477e42156db
branch: master
repository: bevy-lightyear-template
topic: "Migrating bevy_triplanar_splatting from Bevy 0.14 to 0.17"
tags: [research, migration, bevy, bevy_triplanar_splatting, material, shader, wgsl]
status: complete
last_updated: 2026-02-27
last_updated_by: claude
---

# Research: Migrating bevy_triplanar_splatting from Bevy 0.14 to 0.17

**Date**: 2026-02-27T22:37:08-08:00
**Researcher**: claude
**Git Commit**: da4b04bc61eab50383fb714815905477e42156db
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to migrate `git/bevy_triplanar_splatting/` from Bevy 0.14 to 0.17.

## Summary

The migration spans 3 major Bevy releases (0.14→0.15→0.16→0.17). The crate implements a custom `Material` with `AsBindGroup`, custom vertex attributes, pipeline specialization, and 4 WGSL shaders. The changes fall into: Rust API changes (imports, trait signatures, bundle removal), WGSL shader changes (bind group renumbering, import path changes), and Cargo.toml updates.

Two existing crates in the project already run on Bevy 0.17 and serve as reference patterns: `git/bevy_fsl_box_frame` (simple Material) and `git/bevy_voxel_world` (MaterialExtension with specialize + custom vertex layout).

## Detailed Findings

### File-by-File Migration

---

### 1. Cargo.toml

| Change | Old (0.14) | New (0.17) |
|--------|-----------|------------|
| bevy version | `"0.14.1"` | `"0.17"` |
| Handle constant macro | (N/A) | add `uuid_handle` if importing directly |
| thiserror | `"1"` | `"2"` (Bevy 0.17 uses thiserror 2.x) |
| dev-dependencies bevy | `"0.14.1"` | `"0.17"` |

Features `bevy_asset`, `bevy_core_pipeline`, `bevy_pbr`, `bevy_render`, `tonemapping_luts`, `zstd` all remain valid.

New optional features available: `bevy_camera`, `bevy_light`, `bevy_mesh`, `bevy_shader`.

---

### 2. plugin.rs

**Current code** ([plugin.rs](git/bevy_triplanar_splatting/src/plugin.rs)):

```rust
use bevy::asset::{embedded_asset, load_internal_asset};
const TRIPLANAR_SHADER_HANDLE: Handle<Shader> = Handle::weak_from_u128(2631398565563939187);
const BIPLANAR_SHADER_HANDLE: Handle<Shader> = Handle::weak_from_u128(1945949403120376729);
```

**Changes needed:**

| Line | Old | New | Version |
|------|-----|-----|---------|
| 2 | `use bevy::asset::{embedded_asset, load_internal_asset};` | `use bevy::asset::{embedded_asset, load_internal_asset, uuid_handle};` | 0.16+ |
| 5 | `Handle::weak_from_u128(2631398565563939187)` | `uuid_handle!("uuid-string-here")` | 0.16 (deprecated), 0.17 |
| 6 | `Handle::weak_from_u128(1945949403120376729)` | `uuid_handle!("uuid-string-here")` | 0.16 (deprecated), 0.17 |

**Note on uuid_handle vs weak_handle:** The project's existing 0.17 crates use `uuid_handle!` from `bevy::asset` (see [solid_color_material.rs:10-11](git/bevy_fsl_box_frame/src/solid_color_material.rs#L10-L11)). The migration guides mention `weak_handle!` (0.16), but the actual 0.17 API appears to be `uuid_handle!`.

**`load_internal_asset!`**: Still compiles on 0.17 (used by [bevy_fsl_box_frame](git/bevy_fsl_box_frame/src/lib.rs#L29-L34)), though officially deprecated in favor of `embedded_asset!`. Since this crate already uses both, no macro migration is strictly required.

**`embedded_asset!`**: Unchanged across all versions.

---

### 3. triplanar_material.rs

This file has the most changes.

#### 3a. Imports

**Current** ([triplanar_material.rs:1-13](git/bevy_triplanar_splatting/src/triplanar_material.rs#L1-L13)):
```rust
use bevy::{
    pbr::{MaterialPipeline, MaterialPipelineKey, StandardMaterialFlags},
    prelude::*,
    render::{
        mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef},
        render_asset::RenderAssets,
        render_resource::{
            AsBindGroup, AsBindGroupShaderType, Face, RenderPipelineDescriptor, ShaderRef,
            ShaderType, SpecializedMeshPipelineError, TextureFormat, VertexFormat,
        },
        texture::GpuImage,
    },
};
```

**New (0.17):**
```rust
use bevy::{
    pbr::{MaterialPipeline, MaterialPipelineKey, StandardMaterialFlags},
    prelude::*,
    mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef},
    render::{
        render_asset::RenderAssets,
        render_resource::{
            AsBindGroup, AsBindGroupShaderType, Face, RenderPipelineDescriptor,
            ShaderType, SpecializedMeshPipelineError, TextureFormat, VertexFormat,
        },
        texture::GpuImage,
    },
    shader::ShaderRef,
};
```

Key import changes:
- `ShaderRef` → moved from `bevy::render::render_resource` to `bevy::shader` (0.17)
- `MeshVertexAttribute`, `MeshVertexBufferLayoutRef` → moved from `bevy::render::mesh` to `bevy::mesh` (0.17)
- `RenderAssets<GpuImage>` → unchanged
- `AsBindGroupShaderType` → unchanged at `bevy::render::render_resource`
- `SpecializedMeshPipelineError` → unchanged at `bevy::render::render_resource`
- `StandardMaterialFlags` → unchanged at `bevy::pbr`
- `MaterialPipeline` → still at `bevy::pbr` but **no longer generic** (0.17)

#### 3b. Material trait impl — specialize() signature

**Current** ([triplanar_material.rs:109-131](git/bevy_triplanar_splatting/src/triplanar_material.rs#L109-L131)):
```rust
fn specialize(
    _pipeline: &MaterialPipeline<Self>,
    descriptor: &mut RenderPipelineDescriptor,
    layout: &MeshVertexBufferLayoutRef,
    key: MaterialPipelineKey<Self>,
) -> Result<(), SpecializedMeshPipelineError> {
```

**New (0.17):**
```rust
fn specialize(
    _pipeline: &MaterialPipeline,          // no longer generic
    descriptor: &mut RenderPipelineDescriptor,
    layout: &MeshVertexBufferLayoutRef,
    key: MaterialPipelineKey<Self>,
) -> Result<(), SpecializedMeshPipelineError> {
```

`MaterialPipeline` lost all fields except `mesh_pipeline: MeshPipeline` in 0.17. Since the current code uses `_pipeline` (unused), this is a signature-only change.

#### 3c. Color API

**Current** ([triplanar_material.rs:234](git/bevy_triplanar_splatting/src/triplanar_material.rs#L234)):
```rust
base_color: self.base_color.to_linear().to_vec4(),
emissive: self.emissive.to_srgba().to_vec4(),
```

**No changes needed.** `.to_linear()` was renamed from `.linear()` in 0.14→0.15, but the current code already uses `to_linear()`. `Color::srgb()`, `Color::BLACK` are unchanged.

#### 3d. Everything else in triplanar_material.rs — unchanged

- `#[derive(AsBindGroup, Asset, Reflect, Debug, Clone)]`
- `#[type_path = "..."]`
- `#[bind_group_data(TriplanarMaterialKey)]`
- `#[uniform(0, TriplanarMaterialUniform)]`
- `#[texture(N, dimension = "2d_array")]` / `#[sampler(N)]`
- `AlphaMode` variants
- `Face`
- `ATTRIBUTE_MATERIAL_WEIGHTS` constant
- `TriplanarMaterialKey` struct
- `AsBindGroupShaderType` impl

---

### 4. WGSL Shaders

#### 4a. Fragment shader — Bind group renumbering (0.16→0.17, critical)

**Current** ([triplanar_material_frag.wgsl:39-60](git/bevy_triplanar_splatting/src/shaders/triplanar_material_frag.wgsl#L39-L60)):
```wgsl
@group(2) @binding(0)
var<uniform> material: TriplanarMaterial;
@group(2) @binding(1)
var base_color_texture: texture_2d_array<f32>;
// ... all @group(2)
```

**New (0.17):** Replace all `@group(2)` with `@group(#{MATERIAL_BIND_GROUP})`:
```wgsl
@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var<uniform> material: TriplanarMaterial;
@group(#{MATERIAL_BIND_GROUP}) @binding(1)
var base_color_texture: texture_2d_array<f32>;
// ...
```

The bind group layout was reorganized in 0.17 (wgpu 25): view bindings split across groups 0-1, mesh bindings at group 2, material bindings at group 3. Using `#{MATERIAL_BIND_GROUP}` is the future-proof approach.

**Reference:** [bevy_voxel_world shader](git/bevy_voxel_world/src/shaders/voxel_texture.wgsl) already uses `@group(#{MATERIAL_BIND_GROUP})`.

#### 4b. Fragment shader — SSAO import rename (0.14→0.15)

**Current** ([triplanar_material_frag.wgsl:11](git/bevy_triplanar_splatting/src/shaders/triplanar_material_frag.wgsl#L11)):
```wgsl
#import bevy_pbr::gtao_utils::gtao_multibounce
```

**New (0.15+):**
```wgsl
#import bevy_pbr::ssao_utils::ssao_multibounce
```

GTAO was replaced by VBAO in Bevy 0.15. The utility was renamed accordingly.

#### 4c. Fragment shader — mesh[instance_index] access

**Current** ([triplanar_material_frag.wgsl:192](git/bevy_triplanar_splatting/src/shaders/triplanar_material_frag.wgsl#L192)):
```wgsl
pbr_input.flags = mesh[in.instance_index].flags;
```

This accesses `mesh` from `bevy_pbr::mesh_bindings::mesh`. This storage buffer access pattern may have changed with the mesh binding reorganization in 0.17. Needs verification at compile time.

#### 4d. Fragment shader — PbrInput field changes

**Current** ([triplanar_material_frag.wgsl:165](git/bevy_triplanar_splatting/src/shaders/triplanar_material_frag.wgsl#L165)):
```wgsl
pbr_input.diffuse_occlusion = occlusion;
```

`PbrInput` fields may have been restructured across versions. The `diffuse_occlusion` field needs verification. Some versions split this into `diffuse_occlusion` and `specular_occlusion`.

#### 4e. Fragment shader — fog and tonemapping APIs

**Current** ([triplanar_material_frag.wgsl:200-206](git/bevy_triplanar_splatting/src/shaders/triplanar_material_frag.wgsl#L200-L206)):
```wgsl
if (fog.mode != FOG_MODE_OFF && ...) {
    output_color = pbr_functions::apply_fog(fog, output_color, in.world_position.xyz, view.world_position.xyz);
}
output_color = tone_mapping(output_color, view.color_grading);
```

The `apply_fog` and `tone_mapping` function signatures should be verified at compile time.

#### 4f. All shaders — WGSL constant types (0.16→0.17)

Any bare float constants without explicit types need type annotations:
```wgsl
// Old
const FOO = 1.0;
// New
const FOO: f32 = 1.0;
```

The existing shaders don't appear to have bare constants, but verify during compilation.

#### 4g. Shader import paths — mostly stable

| Import | Status across 0.14→0.17 |
|--------|------------------------|
| `bevy_pbr::mesh_functions` | Unchanged |
| `bevy_pbr::view_transformations` | Unchanged |
| `bevy_pbr::pbr_functions` | Unchanged |
| `bevy_pbr::pbr_types` | Unchanged |
| `bevy_pbr::mesh_bindings::mesh` | Unchanged |
| `bevy_pbr::mesh_view_bindings::{view, fog}` | Unchanged |
| `bevy_pbr::mesh_view_types::FOG_MODE_OFF` | Unchanged |
| `bevy_core_pipeline::tonemapping` | Unchanged |
| `bevy_render::maths::powsafe` | Unchanged |
| `bevy_pbr::gtao_utils::gtao_multibounce` | **Renamed** → `ssao_utils::ssao_multibounce` (0.15) |

---

### 5. Example (render.rs)

The example has many changes but is not required for the library to compile:

| Change | Old (0.14) | New (0.17) | Version |
|--------|-----------|------------|---------|
| Mesh+Material spawning | `MaterialMeshBundle { mesh, material, ..default() }` | `(Mesh3d(mesh), MeshMaterial3d(material))` | 0.15 |
| Point light | `PointLightBundle { point_light: PointLight { .. }, ..default() }` | `(PointLight { .. }, Transform::from_xyz(..))` | 0.15 |
| Camera | `Camera3dBundle::default()` | `Camera3d::default()` | 0.15 |
| Time | `time.elapsed_seconds()` | `time.elapsed_secs()` | 0.15 |
| Image imports | `bevy::render::texture::{ImageAddressMode, ...}` | `bevy::image::{ImageAddressMode, ...}` | 0.15 |
| Mesh creation | `Mesh::try_from(Sphere::new(5.0).mesh().ico(6).unwrap()).unwrap()` | Verify `Mesh::try_from` still valid | — |
| smooth-bevy-cameras | `"0.12.0"` | Needs 0.17-compatible version or removal | — |

---

## Migration Checklist (Ordered)

### Phase 1: Cargo.toml
- [ ] `bevy = "0.14.1"` → `"0.17"`
- [ ] `thiserror = "1"` → `"2"`
- [ ] dev-dependencies `bevy = "0.14.1"` → `"0.17"`
- [ ] dev-dependencies `smooth-bevy-cameras` → find 0.17-compatible version or remove

### Phase 2: Rust — plugin.rs
- [ ] Replace `Handle::weak_from_u128(N)` with `uuid_handle!("uuid-string")`
- [ ] Add `use bevy::asset::uuid_handle;` import

### Phase 3: Rust — triplanar_material.rs
- [ ] Move `ShaderRef` import to `bevy::shader::ShaderRef`
- [ ] Move `MeshVertexAttribute`, `MeshVertexBufferLayoutRef` to `bevy::mesh::`
- [ ] Change `specialize()` signature: `&MaterialPipeline<Self>` → `&MaterialPipeline`

### Phase 4: WGSL — triplanar_material_frag.wgsl
- [ ] Replace all `@group(2)` with `@group(#{MATERIAL_BIND_GROUP})`
- [ ] Replace `gtao_utils::gtao_multibounce` with `ssao_utils::ssao_multibounce`
- [ ] Verify `mesh[in.instance_index].flags` still works
- [ ] Verify `PbrInput` field names (`diffuse_occlusion`, etc.)
- [ ] Verify `apply_fog` signature
- [ ] Verify `tone_mapping` signature
- [ ] Verify `apply_pbr_lighting` signature
- [ ] Verify `premultiply_alpha` signature
- [ ] Add explicit types to any bare WGSL constants

### Phase 5: WGSL — triplanar_material_vert.wgsl
- [ ] Verify `mesh_functions::get_world_from_local` still takes `instance_index`
- [ ] Verify `mesh_functions::mesh_normal_local_to_world` signature
- [ ] Verify `mesh_functions::mesh_position_local_to_world` signature

### Phase 6: Example (render.rs) — optional
- [ ] Replace `MaterialMeshBundle` with `Mesh3d` + `MeshMaterial3d`
- [ ] Replace `PointLightBundle` with `PointLight` component
- [ ] Replace `Camera3dBundle` with `Camera3d` component
- [ ] Replace `time.elapsed_seconds()` with `time.elapsed_secs()`
- [ ] Update image imports to `bevy::image`
- [ ] Update or remove `smooth-bevy-cameras` dependency

### Phase 7: Compile and test
- [ ] `cargo check` the crate
- [ ] Fix any remaining shader compilation errors at runtime
- [ ] Test the example if applicable

## Reference Implementations in Project

| Crate | Pattern | Bevy | Key file |
|-------|---------|------|----------|
| `git/bevy_fsl_box_frame` | Simple Material + `load_internal_asset!` + `uuid_handle!` | 0.17 | [solid_color_material.rs](git/bevy_fsl_box_frame/src/solid_color_material.rs) |
| `git/bevy_voxel_world` | MaterialExtension + `specialize()` + custom vertex + `uuid_handle!` | 0.17 | [voxel_material.rs](git/bevy_voxel_world/src/voxel_material.rs) |

## Risk Areas

1. **Fragment shader complexity**: The frag shader manually constructs `PbrInput` and calls `apply_pbr_lighting`, `apply_fog`, `tone_mapping`. These internal Bevy shader APIs are the most likely to have changed signatures across 3 major versions. Expect iterative fixes during compilation.

2. **`mesh[in.instance_index]` access**: The mesh storage buffer binding may have changed with the bind group reorganization. May need to use `bevy_render::instance_index::get_instance_index` helper.

3. **SSAO / screen_space_ambient_occlusion_texture**: The SSAO system was reworked (GTAO → VBAO). The texture binding and guard macros may need updating.

4. **StandardMaterialFlags in shader**: The crate references `StandardMaterialFlags` both in Rust (for `flags.bits()`) and WGSL (for `STANDARD_MATERIAL_FLAGS_*` constants). The Rust-side type and WGSL constants appear unchanged, but the exact bit values should be verified.

## Migration Sources

- [Bevy 0.14→0.15 Migration Guide](https://bevy.org/learn/migration-guides/0-14-to-0-15/)
- [Bevy 0.15→0.16 Migration Guide](https://bevy.org/learn/migration-guides/0-15-to-0-16/)
- [Bevy 0.16→0.17 Migration Guide](https://bevy.org/learn/migration-guides/0-16-to-0-17/)
- [Material trait docs (0.17)](https://docs.rs/bevy/0.17.2/bevy/pbr/trait.Material.html)
- [MaterialPipeline docs (0.17)](https://docs.rs/bevy/0.17.2/bevy/pbr/struct.MaterialPipeline.html)
- [ShaderRef docs (0.17)](https://docs.rs/bevy/0.17.2/bevy/shader/enum.ShaderRef.html)

## Open Questions

1. What is the exact `PbrInput` struct layout in Bevy 0.17 WGSL? Fields like `diffuse_occlusion`, `specular_occlusion`, `frag_coord`, `world_position`, `world_normal`, `N`, `V`, `is_orthographic`, `flags` need verification.
2. Does `mesh[in.instance_index].flags` still work, or does it need `get_instance_index(in.instance_index)`?
3. Does `apply_fog` still take `(fog, color, world_pos, view_pos)` or has the signature changed?
4. Is there a 0.17-compatible version of `smooth-bevy-cameras`?
5. What UUID strings correspond to the u128 values `2631398565563939187` and `1945949403120376729`?
