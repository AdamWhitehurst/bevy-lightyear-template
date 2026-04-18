# Research Questions

## Context
Focus on the sprite rig system (`crates/sprite_rig/`), the sprite rig billboard material (`crates/protocol/src/billboard/sprite_rig_material.rs` + `assets/shaders/sprite_rig_billboard.wgsl`), and sprite asset loading. Secondary areas include other `ExtendedMaterial` implementations in the project, existing palette/color-table systems (voxel chunks, `.vox` models), and the RON asset formats for rigs/animsets.

## Questions
1. How is `SpriteRigMaterial` defined and registered, what uniforms/bindings does `SpriteRigBillboardExt` declare on the Rust side, and how are those bindings mirrored (group/binding indices, types) in `sprite_rig_billboard.wgsl`?

2. Trace rig spawning end-to-end in `crates/sprite_rig/src/spawn.rs`: how are bone/slot entities created from a `SpriteRigAsset`, how are meshes built for sprite attachments, and how is `SpriteRigMaterial` instantiated and assigned to each bone entity?

3. How does the `SpriteImageHandles` resource load and store sprite PNGs, and how are individual images (torso, head, leg, arm) resolved to specific bone/slot attachments via the rig asset definition?

4. What is the pixel format and color content of the sprite PNGs under `assets/sprites/humanoid/` — full-color RGBA artwork, grayscale, single-channel indexed/index-map, or something else — and how is that format reflected in the `Image` asset when loaded?

5. How do the RON asset formats (`rig.ron`, `animset.ron`, `anim.ron`) declare bones, slots, and attachments, and which struct fields in `asset.rs` would be the natural place to add per-bone/per-slot metadata (e.g., a color or palette-index field)?

6. What patterns exist in this project for propagating per-entity component data into a material — e.g., a component changes, and its value reaches a material uniform, a material handle swap, or a per-instance attribute?

7. How do the three existing `ExtendedMaterial` extensions in the project (`BillboardExt`, `SpriteRigBillboardExt`, `ShadowOnlyExt`) declare additional textures, samplers, uniform structs, or array/struct uniforms beyond standard PBR, and what is the idiomatic pattern for adding a new one?

8. How does the voxel `PalettedChunk` system (`crates/voxel_map_engine/src/palette.rs`) and the `.vox` model color-table code (`crates/protocol/src/vox_model/meshing.rs`, `lod.rs`) represent a palette and map indices to RGB/RGBA colors at both the data-structure and rendering level?

9. How does the `protocol` crate treat components and materials for networking/replication — are material-affecting components replicated, and would a new `SpriteRigPalette` component need to live in `protocol` or in `sprite_rig`?
