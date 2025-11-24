use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use protocol::Message1;
use std::time::Duration;

#[test]
fn test_message_sender_component() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins {
        tick_duration: Duration::from_millis(16),
    });
    app.add_plugins(protocol::ProtocolPlugin);

    // Spawn client entity
    let client_id = app.world_mut().spawn_empty().id();

    // Add MessageSender component
    app.world_mut().entity_mut(client_id).insert(MessageSender::<Message1>::default());

    app.update();

    // Verify MessageSender present
    let sender = app.world().get::<MessageSender<Message1>>(client_id);
    assert!(sender.is_some(), "MessageSender<Message1> not present");
}

#[test]
fn test_message_receiver_component() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins {
        tick_duration: Duration::from_millis(16),
    });
    app.add_plugins(protocol::ProtocolPlugin);

    // Spawn client entity
    let client_id = app.world_mut().spawn_empty().id();

    // Add MessageReceiver component
    app.world_mut().entity_mut(client_id).insert(MessageReceiver::<Message1>::default());

    app.update();

    // Verify MessageReceiver present
    let receiver = app.world().get::<MessageReceiver<Message1>>(client_id);
    assert!(receiver.is_some(), "MessageReceiver<Message1> not present");
}
