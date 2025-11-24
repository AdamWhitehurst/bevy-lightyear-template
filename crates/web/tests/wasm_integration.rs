#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn test_wasm_panic_hook() {
    console_error_panic_hook::set_once();
    assert_eq!(2 + 2, 4);
}

#[wasm_bindgen_test]
fn test_protocol_imports() {
    use protocol::Message1;

    let msg = Message1(42);
    assert_eq!(msg.0, 42);
}

#[wasm_bindgen_test]
fn test_bevy_minimal_app() {
    use bevy::prelude::*;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    // Spawn a test entity
    app.world_mut().spawn_empty();

    // Run one update cycle
    app.update();

    // Verify app is functional (has at least one entity)
    assert!(app.world().entities().len() > 0, "App should have spawned entity");
}
