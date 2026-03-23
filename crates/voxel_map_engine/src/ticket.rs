use bevy::prelude::*;

/// The ticket type determines base level and semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TicketType {
    /// Full simulation around players. Base level 0.
    Player,
    /// NPC simulation. Base level 1.
    Npc,
    /// Temporary ticket for pre-loading destination during map transitions. Base level 2.
    MapTransition,
}

impl TicketType {
    /// The base load level for this ticket type. Lower = stronger.
    pub fn base_level(self) -> u32 {
        match self {
            TicketType::Player => 0,
            TicketType::Npc => 1,
            TicketType::MapTransition => 2,
        }
    }

    /// The default Chebyshev radius for this ticket type.
    pub fn default_radius(self) -> u32 {
        match self {
            TicketType::Player => 100,
            TicketType::Npc => 1,
            TicketType::MapTransition => 4,
        }
    }
}

/// Attach to entities whose `GlobalTransform` drives chunk loading for a specific map.
/// Replaces `ChunkTarget`. Local-only — not replicated over the network.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ChunkTicket {
    /// Which map this ticket loads chunks for.
    pub map_entity: Entity,
    /// Ticket type determines the base load level.
    pub ticket_type: TicketType,
    /// Radius in chunks (Chebyshev 2D) that this ticket influences.
    /// Effective level at distance d = base_level + d.
    pub radius: u32,
}

impl ChunkTicket {
    pub fn new(map_entity: Entity, ticket_type: TicketType, radius: u32) -> Self {
        debug_assert!(
            map_entity != Entity::PLACEHOLDER,
            "ChunkTicket::new called with Entity::PLACEHOLDER"
        );
        Self {
            map_entity,
            ticket_type,
            radius,
        }
    }

    /// Player ticket with default radius (10).
    pub fn player(map_entity: Entity) -> Self {
        Self::new(
            map_entity,
            TicketType::Player,
            TicketType::Player.default_radius(),
        )
    }

    /// NPC ticket with default radius (1).
    pub fn npc(map_entity: Entity) -> Self {
        Self::new(
            map_entity,
            TicketType::Npc,
            TicketType::Npc.default_radius(),
        )
    }

    /// Map transition ticket with default radius (4).
    pub fn map_transition(map_entity: Entity) -> Self {
        Self::new(
            map_entity,
            TicketType::MapTransition,
            TicketType::MapTransition.default_radius(),
        )
    }
}

/// A chunk column's load state, derived from its effective level.
///
/// This plan uses `LOAD_LEVEL_THRESHOLD` for loaded/unloaded decisions.
/// `LoadState` variants are used for debug display and will drive
/// simulation zone differentiation in a future plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LoadState {
    /// Level 0: Full simulation — entity AI, physics, spawning.
    EntityTicking,
    /// Level 1: NPC simulation only, player entities frozen.
    BlockTicking,
    /// Level 2: Data loaded, meshed, no simulation. Available for neighbor padding.
    Border,
    /// Level 3+: Generation in progress, not accessible for gameplay.
    Inaccessible,
}

impl LoadState {
    /// Derive load state from an effective level.
    pub fn from_level(level: u32) -> Self {
        match level {
            0 => LoadState::EntityTicking,
            1 => LoadState::BlockTicking,
            2 => LoadState::Border,
            _ => LoadState::Inaccessible,
        }
    }

    /// The maximum level that produces this load state.
    pub fn max_level(self) -> u32 {
        match self {
            LoadState::EntityTicking => 0,
            LoadState::BlockTicking => 1,
            LoadState::Border => 2,
            LoadState::Inaccessible => u32::MAX,
        }
    }
}

/// The threshold level at or below which a column is considered "loaded"
/// (data in octree, mesh spawned). Columns above this level are unloaded.
///
/// Border (level 2) is the weakest loaded state — chunks at Border have data
/// and meshes but no simulation.
#[cfg(test)]
pub const LOAD_LEVEL_THRESHOLD: u32 = 100;
#[cfg(not(test))]
pub const LOAD_LEVEL_THRESHOLD: u32 = 40;

/// Maximum level value. Columns beyond this are not tracked by the propagator.
pub const MAX_LEVEL: u32 = 640;

/// Default column height range: 16 chunks vertically (Y range −8 to 7, exclusive upper bound).
pub const DEFAULT_COLUMN_Y_MIN: i32 = -8;
pub const DEFAULT_COLUMN_Y_MAX: i32 = 8;

/// Expand a 2D column position to all 3D chunk positions in the column.
/// Uses exclusive upper bound: `y_min..y_max`.
pub fn column_to_chunks(col: IVec2, y_min: i32, y_max: i32) -> impl Iterator<Item = IVec3> {
    (y_min..y_max).map(move |y| IVec3::new(col.x, y, col.y))
}

/// Convert a 3D chunk position to its 2D column (drop Y).
pub fn chunk_to_column(chunk_pos: IVec3) -> IVec2 {
    IVec2::new(chunk_pos.x, chunk_pos.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_type_base_levels() {
        assert_eq!(TicketType::Player.base_level(), 0);
        assert_eq!(TicketType::Npc.base_level(), 1);
        assert_eq!(TicketType::MapTransition.base_level(), 2);
    }

    #[test]
    fn ticket_type_default_radii() {
        assert_eq!(TicketType::Player.default_radius(), 10);
        assert_eq!(TicketType::Npc.default_radius(), 1);
        assert_eq!(TicketType::MapTransition.default_radius(), 4);
    }

    #[test]
    fn load_state_from_level() {
        assert_eq!(LoadState::from_level(0), LoadState::EntityTicking);
        assert_eq!(LoadState::from_level(1), LoadState::BlockTicking);
        assert_eq!(LoadState::from_level(2), LoadState::Border);
        assert_eq!(LoadState::from_level(3), LoadState::Inaccessible);
        assert_eq!(LoadState::from_level(100), LoadState::Inaccessible);
    }

    #[test]
    fn column_to_chunks_produces_correct_range() {
        let col = IVec2::new(3, 5);
        let chunks: Vec<IVec3> = column_to_chunks(col, -2, 2).collect();
        assert_eq!(chunks.len(), 4); // -2, -1, 0, 1 (exclusive upper)
        assert_eq!(chunks[0], IVec3::new(3, -2, 5));
        assert_eq!(chunks[3], IVec3::new(3, 1, 5));
    }

    #[test]
    fn chunk_to_column_drops_y() {
        assert_eq!(chunk_to_column(IVec3::new(1, 99, 2)), IVec2::new(1, 2));
    }

    /// Create a dummy entity for tests (not PLACEHOLDER).
    fn test_entity() -> Entity {
        Entity::from_raw_u32(999).expect("valid test entity")
    }

    #[test]
    fn convenience_constructors_use_default_radii() {
        let e = test_entity();
        let p = ChunkTicket::player(e);
        assert_eq!(p.ticket_type, TicketType::Player);
        assert_eq!(p.radius, 10);
        let n = ChunkTicket::npc(e);
        assert_eq!(n.ticket_type, TicketType::Npc);
        assert_eq!(n.radius, 1);
        let t = ChunkTicket::map_transition(e);
        assert_eq!(t.ticket_type, TicketType::MapTransition);
        assert_eq!(t.radius, 4);
    }

    #[test]
    fn new_allows_custom_radius() {
        let e = test_entity();
        let t = ChunkTicket::new(e, TicketType::Player, 20);
        assert_eq!(t.radius, 20);
    }
}
