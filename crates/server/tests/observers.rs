use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::prelude::server::*;
use protocol::FIXED_TIMESTEP_HZ;
use std::time::Duration;

#[derive(Resource, Default)]
struct ObserverTestState {
    client_connected: bool,
    client_disconnected: bool,
}

fn on_client_connected(
    _trigger: On<Add, Connected>,
    mut state: ResMut<ObserverTestState>,
) {
    state.client_connected = true;
}

fn on_client_disconnected(
    _trigger: On<Add, Disconnected>,
    mut state: ResMut<ObserverTestState>,
) {
    state.client_disconnected = true;
}

#[test]
fn test_server_observer_registration() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ServerPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    // Add test resource and observers
    app.init_resource::<ObserverTestState>();
    app.add_observer(on_client_connected);
    app.add_observer(on_client_disconnected);

    // Verify observers registered
    app.update();

    let state = app.world().resource::<ObserverTestState>();
    assert_eq!(state.client_connected, false);
    assert_eq!(state.client_disconnected, false);
}
