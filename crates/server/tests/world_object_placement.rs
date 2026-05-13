use std::collections::HashMap;

use avian3d::prelude::Position;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::reflect::PartialReflect;
use lightyear::prelude::{NetworkVisibility, Replicate, ReplicationSendPlugin};
use protocol::map::{ChunkEntityRef, MapInstanceId};
use protocol::vox_model::{VoxModelAsset, VoxModelRegistry};
use protocol::world_object::{
    PlacementOffset, WorldObjectDef, WorldObjectDefRegistry, WorldObjectId,
    WorldObjectPlacementRejectReason, WorldObjectPlacementRequest,
};
use voxel_map_engine::prelude::{
    chunk_to_column, ChunkData, ChunkStatus, MapDimensions, VoxelMapInstance, WorldVoxel,
};

const CHUNK_SIZE: u32 = 16;
const PADDED_VOLUME_16: usize = 18 * 18 * 18;

fn object_id() -> WorldObjectId {
    WorldObjectId("test_tree".to_string())
}

fn test_def(offset: Vec3) -> WorldObjectDef {
    WorldObjectDef {
        components: vec![Box::new(PlacementOffset(offset)) as Box<dyn PartialReflect>],
    }
}

fn test_registry() -> WorldObjectDefRegistry {
    WorldObjectDefRegistry {
        objects: HashMap::from([(object_id(), test_def(Vec3::new(0.0, 1.5, 0.0)))]),
    }
}

fn dimensions(bounds: Option<IVec3>) -> MapDimensions {
    MapDimensions {
        chunk_size: CHUNK_SIZE,
        column_y_range: (-4, 4),
        tree_height: 5,
        bounds,
    }
}

fn loaded_instance(chunk_pos: IVec3) -> VoxelMapInstance {
    let mut instance = VoxelMapInstance::new(5, CHUNK_SIZE);
    let voxels = vec![WorldVoxel::Air; PADDED_VOLUME_16];
    instance.insert_chunk_data(
        chunk_pos,
        ChunkData::from_voxels(&voxels, ChunkStatus::Full),
    );
    instance.chunk_levels.insert(chunk_to_column(chunk_pos), 0);
    instance
}

fn placement_request(base_position: Vec3) -> WorldObjectPlacementRequest {
    WorldObjectPlacementRequest {
        sequence: 7,
        object_id: object_id(),
        base_position,
    }
}

#[test]
fn accepted_placement_spawns_replicated_chunk_entity() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ReplicationSendPlugin);
    app.register_type::<PlacementOffset>();
    app.insert_resource(VoxModelRegistry {
        models: HashMap::new(),
    });
    app.init_resource::<Assets<VoxModelAsset>>();
    app.init_resource::<Assets<Mesh>>();

    let base_position = Vec3::new(1.0, 2.0, 3.0);
    let offset = Vec3::new(0.0, 1.5, 0.0);
    let final_position = base_position + offset;
    let def = test_def(offset);
    let map_entity = app.world_mut().spawn_empty().id();
    let map_id = MapInstanceId::Overworld;

    app.world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  type_registry: Res<AppTypeRegistry>,
                  vox_registry: Res<VoxModelRegistry>,
                  vox_assets: Res<Assets<VoxModelAsset>>,
                  meshes: Res<Assets<Mesh>>| {
                server::world_object::spawn_placed_world_object(
                    &mut commands,
                    object_id(),
                    &def,
                    base_position,
                    map_entity,
                    map_id.clone(),
                    CHUNK_SIZE,
                    &type_registry,
                    &vox_registry,
                    &vox_assets,
                    &meshes,
                );
            },
        )
        .unwrap();

    let mut query = app.world_mut().query::<(
        &WorldObjectId,
        &MapInstanceId,
        &Position,
        &ChunkEntityRef,
        &Replicate,
        &NetworkVisibility,
    )>();
    let objects: Vec<_> = query.iter(app.world()).collect();

    assert_eq!(objects.len(), 1);
    let (spawned_id, spawned_map_id, position, chunk_ref, _, _) = objects[0];
    assert_eq!(spawned_id, &object_id());
    assert_eq!(spawned_map_id, &MapInstanceId::Overworld);
    assert_eq!(position.0, final_position);
    assert_eq!(chunk_ref.map_entity, map_entity);
    assert_eq!(
        chunk_ref.chunk_pos,
        voxel_map_engine::lifecycle::world_to_chunk_pos(final_position, CHUNK_SIZE)
    );
}

#[test]
fn accepted_placement_validation_returns_final_position_and_chunk() {
    let base_position = Vec3::new(1.0, 2.0, 3.0);
    let expected_final = Vec3::new(1.0, 3.5, 3.0);
    let chunk_pos = voxel_map_engine::lifecycle::world_to_chunk_pos(expected_final, CHUNK_SIZE);
    let instance = loaded_instance(chunk_pos);
    let dims = dimensions(None);
    let registry = test_registry();

    let (_, final_position, validated_chunk) = server::map::validate_world_object_placement(
        &placement_request(base_position),
        &instance,
        &dims,
        &registry,
    )
    .expect("placement should be valid");

    assert_eq!(final_position, expected_final);
    assert_eq!(validated_chunk, chunk_pos);
}

#[test]
fn rejected_placement_spawns_no_entity() {
    let base_position = Vec3::new(1.0, 2.0, 3.0);
    let expected_final = Vec3::new(1.0, 3.5, 3.0);
    let loaded_chunk = voxel_map_engine::lifecycle::world_to_chunk_pos(expected_final, CHUNK_SIZE);
    let loaded = loaded_instance(loaded_chunk);
    let dims = dimensions(Some(IVec3::splat(8)));
    let registry = test_registry();

    let unknown = WorldObjectPlacementRequest {
        object_id: WorldObjectId("missing".to_string()),
        ..placement_request(base_position)
    };
    assert_eq!(
        server::map::validate_world_object_placement(&unknown, &loaded, &dims, &registry)
            .unwrap_err(),
        WorldObjectPlacementRejectReason::UnknownObject
    );

    let non_finite = placement_request(Vec3::new(f32::NAN, 0.0, 0.0));
    assert_eq!(
        server::map::validate_world_object_placement(&non_finite, &loaded, &dims, &registry)
            .unwrap_err(),
        WorldObjectPlacementRejectReason::NonFinitePosition
    );

    let out_of_bounds = placement_request(Vec3::new(200.0, 0.0, 0.0));
    assert_eq!(
        server::map::validate_world_object_placement(&out_of_bounds, &loaded, &dims, &registry)
            .unwrap_err(),
        WorldObjectPlacementRejectReason::OutOfBounds
    );

    let unavailable = placement_request(base_position + Vec3::new(32.0, 0.0, 0.0));
    assert_eq!(
        server::map::validate_world_object_placement(&unavailable, &loaded, &dims, &registry)
            .unwrap_err(),
        WorldObjectPlacementRejectReason::ChunkUnavailable
    );

    assert_eq!(
        WorldObjectPlacementRejectReason::NoControlledCharacter,
        WorldObjectPlacementRejectReason::NoControlledCharacter
    );
}
