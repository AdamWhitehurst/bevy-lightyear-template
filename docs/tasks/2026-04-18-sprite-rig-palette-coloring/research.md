# Research Findings

## Q1: `SpriteRigMaterial` definition, bindings, and shader mirror

### Findings
- Type alias: `pub type SpriteRigMaterial = ExtendedMaterial<StandardMaterial, SpriteRigBillboardExt>` — `crates/protocol/src/billboard/sprite_rig_material.rs:10`
- `SpriteRigBillboardExt` is a **zero-field** struct (`{}`) deriving `AsBindGroup, Asset, TypePath, Clone, Default` — `sprite_rig_material.rs:13-14`
- **No `#[uniform]`, `#[texture]`, `#[sampler]` attributes** — extension declares zero bindings
- `MaterialExtension` impl — `sprite_rig_material.rs:16-31`:
  - `vertex_shader()` → `"shaders/sprite_rig_billboard.wgsl"`
  - `enable_prepass()` → `false`
  - `enable_shadows()` → `false`
- Registered: `app.add_plugins(bevy::pbr::MaterialPlugin::<SpriteRigMaterial>::default())` — `crates/render/src/lib.rs:33`
- WGSL (`assets/shaders/sprite_rig_billboard.wgsl:1-9`): imports `bevy_pbr::{mesh_functions, skinning, forward_io::{Vertex, VertexOutput}, mesh_view_bindings::view}`. Single `@vertex fn vertex` entry point. **No `@group`/`@binding` declarations.**
- Rust↔WGSL mirror: both sides declare zero extension bindings. Skinning and standard PBR bindings come from `bevy_pbr` imports.

## Q2: Rig spawning end-to-end (`crates/sprite_rig/src/spawn.rs`)

### Findings

**Trigger and setup** — `spawn_sprite_rigs` fires on `Added<SpriteRig>` at `spawn.rs:131`.
- `build_slot_lookup(rig)` — `spawn.rs:580-598` — builds `HashMap<&str, SlotInfo>` keyed by `slot.bone`; each `SlotInfo` holds `z_order`, `size`, `anchor`, `image_path` from `AttachmentDef`.
- `topological_sort_bones` — `spawn.rs:601-630` — ensures parents come before children; panics via `debug_assert!` on cycles.

**Bone entity creation** (`spawn_joints`, `spawn.rs:522-555`):
- `bone_transform_from_def` (`spawn.rs:558-577`) builds `Transform` with XY from `BoneDef.default_transform`, Z from `slot_info.z_order` (or 0.0), Z-rotation from degrees, XY scale from def (Z=1.0).
- Spawns entity with only `Name` + `Transform`. **No mesh, no material on bone entities.**
- Parent resolved via `bone_map`; root-less bones parented to `joint_root_id`.
- `add_child` at `spawn.rs:549` inserts the `ChildOf` relation.

**Mesh construction** (`build_rig_mesh_assets`, `spawn.rs:235-281`):
- Cached in `RigMeshCache` keyed by `AssetId<SpriteRigAsset>`.
- `build_texture_atlas` (`spawn.rs:335-374`): collects visible bones, fetches per-bone `Handle<Image>` from `SpriteImageHandles` by `slot_info.image_path`, calls `TextureAtlasBuilder::add_texture`, `builder.build()` produces `(layout, sources, atlas_image)`. Sets `atlas_image.sampler = ImageSampler::nearest()` (`spawn.rs:363`). Returns `HashMap<&str, Rect>` of UV rects keyed by bone name.
- `build_rig_mesh` (`spawn.rs:389-446`): sorts visible bones by `z_order` ascending; for each bone calls `append_quad_vertices` + `append_quad_indices`. `append_quad_vertices` (`spawn.rs:464-497`) pushes 4 positions (Z=0), 4 `[0,0,1]` normals, 4 UV corners sampled from the atlas rect, and `joint_indices = [joint_idx, 0, 0, 0]` / `joint_weights = [1.0, 0, 0, 0]` per vertex — **100% weight to a single controlling bone**.
- Attributes: `POSITION`, `NORMAL`, `UV_0`, `JOINT_INDEX (Uint16x4)`, `JOINT_WEIGHT`, `Indices::U32`. `PrimitiveTopology::TriangleList`, `RenderAssetUsages::RENDER_WORLD`.
- `anchor_y_offset` (`spawn.rs:500-506`): `Center` → 0.0, `TopCenter` → `-size.y/2`, `BottomCenter` → `+size.y/2`.

**Material instantiation** (`spawn.rs:254-265`):
```rust
materials.add(SpriteRigMaterial {
    base: StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(atlas_handle),
        unlit: true, double_sided: true,
        cull_mode: None, alpha_mode: AlphaMode::Blend, ..
    },
    extension: SpriteRigBillboardExt {},
})
```
- **One** `Handle<SpriteRigMaterial>` per rig asset — shared across all character instances.
- Assigned at `spawn.rs:191-201` on the skinned mesh child via `MeshMaterial3d(material_handle)`.
- **No per-bone material.** Bones are differentiated via UV rects only.

**Hierarchy**:
```
character_entity
├── JointRoot (Transform::default)
│   └── sorted bone entities (parent/child via BoneDef.parent)
└── SkinnedMesh child (Mesh3d, MeshMaterial3d, SkinnedMesh.joints = sorted_bones, NotShadowCaster)
ShadowTwin entities (world-root, ShadowTwinOf(joint_entity))
```
- `BoneEntities(bone_map)` inserted on character — `spawn.rs:205`.
- `SkinnedMesh.joints: Vec<Entity>` — topological order matches `vec![Mat4::IDENTITY; n]` inverse-bind array at `spawn.rs:196,251`.
- Shadow twins spawned at world root with per-bone `Rectangle` mesh + `ShadowOnlyMaterial` — `spawn.rs:222-230,310`.

## Q3: `SpriteImageHandles` resource and lookup chain

### Findings
- `pub struct SpriteImageHandles(pub HashMap<String, Handle<Image>>)` — `spawn.rs:72-74`. Key = raw asset path string (e.g., `"sprites/humanoid/head.png"`).
- `init_resource` — `crates/sprite_rig/src/lib.rs:32`.
- `load_rig_sprite_images` runs every `Update` — `lib.rs:43`, body at `spawn.rs:104-128`:
  - Iterates `RigRegistry`; `continue`s with `trace!` if rig asset not yet loaded.
  - Gets `rig.skins["default"]`; for each `AttachmentDef`, calls `asset_server.load::<Image>(&attachment.image)` — `spawn.rs:123`.
  - Registers with `TrackedAssets` — `spawn.rs:124`; inserts into map keyed by same path — `spawn.rs:125`.
- **Lookup chain** (bone → image):
  1. `humanoid.rig.ron` `skins.default` = `HashMap<String, AttachmentDef>` keyed by attachment name (e.g. `"head_default"`).
  2. `SlotDef.default_attachment = "head_default"` → `attachment.image = "sprites/humanoid/head.png"`.
  3. `build_slot_lookup` stores `SlotInfo { image_path, .. }` keyed by `slot.bone` (`spawn.rs:580-598`).
  4. `build_texture_atlas` (`spawn.rs:347-353`) and `build_shadow_part_assets` (`spawn.rs:298-307`) look up `sprite_images.0.get(&slot_info.image_path)` to get `Handle<Image>`.
  5. Atlas builder packs → UV `Rect` per bone → `append_quad_vertices` writes UVs into mesh — `spawn.rs:486-491`.

## Q4: Sprite PNG pixel format

### Findings
- `file` output on `assets/sprites/humanoid/*.png`:
  - `arm.png`: `PNG image data, 32 x 32, 8-bit colormap, non-interlaced`
  - `head.png`: `PNG image data, 80 x 80, 8-bit colormap, non-interlaced`
  - `leg.png`: `PNG image data, 32 x 32, 8-bit colormap, non-interlaced`
  - `torso.png`: `PNG image data, 64 x 80, **1-bit colormap**, non-interlaced`
- **All sprites are indexed-color PNGs (PLTE + tRNS chunks), not RGBA artwork.** `torso.png` is uniquely 1-bit (2-entry palette).
- Load path (`spawn.rs:123`): plain `asset_server.load::<Image>(&attachment.image)` — no `ImageLoaderSettings`, no custom loader, no format hint.
- Bevy's default `ImageLoader` decodes indexed PNGs by **expanding the palette to RGBA8** at load time. The `Image` asset stores decoded RGBA8 pixels; the source palette is lost in decoding.
- Global `DefaultPlugins` usage — `crates/client/src/main.rs:38` — does NOT call `.set(ImagePlugin::default_nearest())`, so global sampler default is linear.
- Nearest sampling is applied only to the runtime-built atlas: `atlas_image.sampler = ImageSampler::nearest()` — `spawn.rs:363`.
- **No code anywhere reads the PNG palette** — there is no palette-preserving loader or custom image format in the codebase.

## Q5: RON asset formats and struct fields

### Findings
- Loader plugin registration — `crates/sprite_rig/src/lib.rs:27-29`:
  - `RonAssetPlugin::<SpriteRigAsset>::new(&["rig.ron"])`
  - `RonAssetPlugin::<SpriteAnimAsset>::new(&["anim.ron"])`
  - `RonAssetPlugin::<SpriteAnimSetAsset>::new(&["animset.ron"])`
- Files: `assets/rigs/humanoid.rig.ron`; `assets/anims/humanoid/humanoid.animset.ron`; `assets/anims/humanoid/{idle,walk,run,punch}.anim.ron`.

**Struct definitions** (`crates/sprite_rig/src/asset.rs`):
```rust
pub struct SpriteRigAsset {       // :7-11
    bones: Vec<BoneDef>,
    slots: Vec<SlotDef>,
    skins: HashMap<String, HashMap<String, AttachmentDef>>,
}
pub struct BoneDef {              // :15-19
    name: String,
    parent: Option<String>,
    default_transform: BoneTransform2d,
}
pub struct BoneTransform2d {      // :23-27
    translation: Vec2, rotation: f32, scale: Vec2,
}
pub struct SlotDef {              // :41-46
    name: String, bone: String, z_order: f32,
    default_attachment: String,
}
pub struct AttachmentDef {        // :50-54
    image: String, anchor: SpriteAnchorDef, size: Vec2,
}
pub enum SpriteAnchorDef { Center, TopCenter, BottomCenter } // :57-63
```

- **No color, tint, or palette-index field exists on any rig struct.**
- Natural places to add per-bone/per-slot palette metadata:
  - `BoneDef` — per-bone color (one per skeletal element).
  - `SlotDef` — per-slot color (independent of bone, useful for swappable gear).
  - `AttachmentDef` — per-attachment color (varies per skin; co-located with image path).

## Q6: Component-to-material propagation patterns

### Findings
Six patterns observed; none mutate a custom `AsBindGroup` uniform field.

1. **`On<Add, Invulnerable>` observer → `materials.get_mut(&handle).base.base_color = color`** — `crates/render/src/health_bar.rs:97-125`. Walks children to find `MeshMaterial3d<BillboardMaterial>`, mutates `StandardMaterial::base_color` in place. Paired `On<Remove, Invulnerable>` restores color.

2. **`On<Add, Health>` observer → `materials.add(...)` spawning new entity with handle** — `crates/render/src/lib.rs:84-104` + `health_bar.rs:38-94`. Creates `BillboardMaterial` inline and inserts `MeshMaterial3d(handle)` on a new child.

3. **`Added<SpriteRig>` system → `materials.add(SpriteRigMaterial { .. })`** — `spawn.rs:131-207,254-265`. Result cached in `RigMeshCache` keyed by `AssetId<SpriteRigAsset>`.

4. **`Changed<Facing>` system → `transform.scale.x = ±1.0`** — `spawn.rs:685-703`. Writes `Transform` of `JointRoot` child; no material involved.

5. **Per-frame `Query<&Health>` → `meshes.get_mut(&handle).insert_attribute(POSITION, ...)`** — `health_bar.rs:159-193`. Mutates mesh vertex positions to resize the FG bar.

6. **`VoxelMapInstance::debug_colors` → conditional `materials.add(...)` vs `handle.clone()`** — `crates/voxel_map_engine/src/lifecycle.rs:915-927,1155-1163`. Handle swap at spawn.

**Pattern summary**:
| Trigger | Mechanism | Target |
|---|---|---|
| `On<Add/Remove, T>` observer | `get_mut + field =` | `StandardMaterial::base_color` |
| `On<Add, T>` observer | `materials.add(...)` spawn | new handle on child |
| `Added<T>` system | `materials.add(...) + cache` | `SpriteRigMaterial` handle |
| `Changed<T>` system | `transform.scale.x =` | `Transform` (not material) |
| Per-frame `Query<&T>` | `meshes.get_mut + insert_attribute` | mesh vertex data |

## Q7: `ExtendedMaterial` extensions — idiomatic pattern

### Findings
All three extensions are **zero-field structs with no bindings**:

| Extension | Uniforms | Textures | Samplers | Shader override | Prepass/Shadows |
|---|---|---|---|---|---|
| `BillboardExt` | none | none | none | vertex | both disabled |
| `SpriteRigBillboardExt` | none | none | none | vertex | both disabled |
| `ShadowOnlyExt` | none | none | none | fragment | both at default (enabled) |

- `BillboardExt` — `crates/protocol/src/billboard/billboard_material.rs:16-32`; vertex_shader → `shaders/billboard.wgsl`.
- `SpriteRigBillboardExt` — `crates/protocol/src/billboard/sprite_rig_material.rs:13-31`; vertex_shader → `shaders/sprite_rig_billboard.wgsl`. WGSL differs from `billboard.wgsl` in Z-rotation extraction (immune to parent Y-rotation — `col0_xz = (world[0].x, world[0].z)`, `raw_sin = world[0].y`, lines 56-58).
- `ShadowOnlyExt` — `crates/protocol/src/billboard/shadow_only_material.rs:21-27`; fragment_shader → `shaders/shadow_only.wgsl`. Shader unconditionally calls `discard` — `shadow_only.wgsl:3-4`.

- **No existing extension declares `#[uniform(N)]`, `#[texture(N)]`, or `#[sampler(N)]`.** Adding any of those would be new territory for this codebase. Bevy convention for `ExtendedMaterial` is to start binding indices at 100 (base `StandardMaterial` occupies 0–19 + view/light groups).

## Q8: Palette systems — voxel chunks and `.vox` models

### Findings

**Part A — `PalettedChunk` (voxel_map_engine)**:
- `PalettedChunk` (`crates/voxel_map_engine/src/palette.rs:22-44`) = CPU-only storage compression enum (`SingleValue`, `Indirect { palette: Vec<WorldVoxel>, data: Vec<u64>, bits_per_entry: u8, .. }`).
- `WorldVoxel` (`voxel_map_engine/src/types.rs:8-12`) = `Air | Unset | Solid(u8)` — the `u8` is a **material/biome ID, not an RGB color**.
- Decode path packs palette indices into u64 words, up to 8 bits per entry — `palette.rs:94-101`.
- **Zero GPU palette upload.** `mesh_chunk_greedy` (`meshing.rs:12-66`) emits only `POSITION`, `NORMAL`, `UV_0` — no color, no palette buffer.
- Chunks render with a flat `StandardMaterial { base_color: srgb(0.5, 0.7, 0.3), .. }` — `lifecycle.rs:122-127`. The `u8` material ID gates greedy-quad merging (`types.rs:164-168`) but never reaches the GPU as color.
- No custom WGSL for chunks.

**Part B — `.vox` model color-table**:
- Parse: `dot_vox::load_bytes` yields `data.palette: Vec<dot_vox::Color>` (256 entries, `{r, g, b, a: u8}`) — `loader.rs:36-47`.
- Per-voxel storage: `VoxModelVoxel::Filled(u8)` holding raw palette index — `vox_model/types.rs:3-8`, `meshing.rs:104-115`.
- **CPU-side resolution at load time**: `palette_color_for_quad` (`meshing.rs:163-183`) looks up `palette[idx]`, converts to linear via `srgb_color_to_linear` (`meshing.rs:196-203`), returns `[f32; 4]`. Applied per-vertex: `colors.extend_from_slice(&[color; 4])` — `meshing.rs:136-143`.
- **GPU representation**: `ATTRIBUTE_COLOR` (linear RGBA f32×4) on the mesh, paired with `StandardMaterial { ..default() }` (white base_color passes vertex colors through) — `meshing.rs:12-14`.
- LOD pipeline preserves palette indices through majority-vote downsampling; each LOD runs its own color resolution — `lod.rs:34-46`.
- No palette texture, no storage buffer, no shader-side palette lookup anywhere in the codebase.

## Q9: Protocol crate and replication placement

### Findings
- `protocol` is the shared gameplay crate — both `client` and `server` depend on it (`crates/client/Cargo.toml:17`, `crates/server/Cargo.toml:22`). **No separate `shared`/`common`/`game_core` crate exists.**
- Component registration: `AppComponentExt::register_component` chained with `.add_prediction()`, `.add_should_rollback(..)`, `.add_linear_correction_fn()`, `.add_linear_interpolation()` — example `protocol/src/lib.rs:197-207`.
- **Existing per-entity color component**: `ColorComponent(pub Color)` — `crates/protocol/src/character/types.rs:108-109`; registered with prediction at `lib.rs:166`. This is the precedent.
- Other replicated components: `MapInstanceId`, `PlayerId`, `CharacterMarker`, `CharacterType`, `Health`, `Invulnerable`, `ActiveAbility`, `Position`, `LinearVelocity` — `lib.rs:157-201`.
- **Material types (`BillboardExt`, `SpriteRigBillboardExt`, `ShadowOnlyExt`) live in `protocol/src/billboard/`** — not replicated (no Serialize/Deserialize, no `register_component`). They live in `protocol` because `protocol` already depends on `bevy_pbr`/`bevy_render` (`protocol/Cargo.toml:11`) and both `client` and `render` need to reference the asset type without creating cycles.
- **Dependency graph**:
  ```
  voxel_map_engine  (standalone)
  protocol          → bevy, avian3d, lightyear, voxel_map_engine, leafwing
  sprite_rig        → protocol, bevy_animation, lightyear
  render            → protocol, sprite_rig, lightyear
  client            → protocol, render, ui, dev
  server            → protocol, persistence
  ```
- `sprite_rig` depends on `protocol`; the reverse is impossible (would cycle).
- **A new replicated `SpriteRigPalette` component belongs in `protocol`** — precedent: `ColorComponent` in `protocol/src/character/types.rs`. Unreplicated per-bone color state could live in `sprite_rig` directly, but any data that must reach clients from the server must be defined in a crate `protocol` can register.

## Cross-Cutting Observations
- **Atlas-based single-material architecture**: the sprite rig uses one merged skinned mesh with per-bone UV rects. Any per-bone variation (color included) either requires the mesh to carry per-vertex data (vertex colors / secondary UVs / joint-indexed lookup) or requires a new bind-group resource indexed by joint ID.
- **No existing `AsBindGroup` uniform mutation pattern**: the three material extensions are empty. All runtime material-driven visual changes in the codebase go through `StandardMaterial::base_color` mutation or handle swaps. Adding a per-bone palette uniform would establish a new pattern.
- **PNG indexed format + Bevy default loader = palette lost at load**: the sprites are authored as indexed-color PNGs, but Bevy decodes them to RGBA8. The source palette does not survive into the `Image` asset. Preserving or reading the palette requires either a custom loader or a separate companion asset.
- **No shader-side palette lookup exists anywhere** — neither voxel chunks nor `.vox` models use one. Voxels ignore color entirely; `.vox` bakes colors to vertex attributes at CPU load time.
- **Protocol is the shared plugin crate**, functioning as the "common" crate. Sprite-rig-specific materials already live there for dependency reasons.

## Open Areas
- The exact PNG palette contents (e.g., which indices correspond to skin/hair/clothing regions) are not documented in the codebase — confirming this would require inspecting the PLTE chunks of each sprite, not visible in source.
- Whether the `png` crate (via Bevy's `ImageLoader`) exposes the source palette through a non-default `CompressedImageFormats`/`ImageLoaderSettings` path was not explored in depth; the codebase uses only default loading.
- The `character.aseprite` file referenced in git status (`M assets/character.aseprite`) may be the authoring source for all humanoid sprites and could reveal the intended palette structure; it was not inspected.
