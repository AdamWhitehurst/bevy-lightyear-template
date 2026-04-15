// Map transition tests removed — the old check_transition_chunks_loaded system
// has been replaced by spatial readiness checking in the client transition state
// machine (client::transition::update_transition_state). The new flow requires
// a full lightyear message pipeline to test meaningfully, which is covered by
// the integration tests in crates/server/tests/integration.rs.
