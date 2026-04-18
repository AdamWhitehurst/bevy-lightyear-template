use bevy::prelude::*;

/// Links a shadow-casting quad to the visible entity whose world position it mirrors.
#[derive(Component)]
pub struct ShadowTwinOf(pub Entity);

/// Positions each shadow twin at its source entity's world location and orients it
/// perpendicular to the directional light. Twins are children of the character
/// entity, so the world-target transform is converted back into the character's
/// local frame via `GlobalTransform::reparented_to` before assignment.
pub fn update_shadow_twins(
    sun_query: Query<&GlobalTransform, With<DirectionalLight>>,
    gt_query: Query<&GlobalTransform, Without<ShadowTwinOf>>,
    mut twin_query: Query<(&ShadowTwinOf, &ChildOf, &mut Transform)>,
) {
    let Ok(sun_gt) = sun_query.single() else {
        trace!("DirectionalLight not yet spawned; shadow twins await sun");
        return;
    };
    let sun_dir = sun_gt.forward();

    for (twin, child_of, mut transform) in &mut twin_query {
        let Ok(source_gt) = gt_query.get(twin.0) else {
            trace!("Shadow twin source entity not yet ready");
            continue;
        };
        let Ok(parent_gt) = gt_query.get(child_of.parent()) else {
            trace!("Shadow twin parent not yet ready");
            continue;
        };

        let desired_world =
            Transform::from_translation(source_gt.translation()).looking_to(sun_dir, Vec3::Y);
        *transform = GlobalTransform::from(desired_world).reparented_to(parent_gt);
    }
}
