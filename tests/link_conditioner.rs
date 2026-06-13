use core::time::Duration;

use bevy::{prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy};
use bevy_replicon::{prelude::*, test_app::ServerTestAppExt};

fn conditioned_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
        LinkConditionerPlugin,
    ));
    app.finish();
    app
}

fn replicated_count(app: &mut App) -> usize {
    app.world_mut().query::<&Remote>().iter(app.world()).len()
}

#[test]
fn default_conditions_pass_through() {
    let mut server_app = conditioned_app();
    let mut client_app = conditioned_app();
    server_app.connect_client(&mut client_app);

    server_app.world_mut().spawn(Replicated);
    server_app.update();
    server_app.exchange_with_client(&mut client_app);
    client_app.update();

    assert_eq!(
        replicated_count(&mut client_app),
        1,
        "a default conditioner must not change replication timing"
    );
}

#[test]
fn latency_holds_then_delivers() {
    let mut server_app = conditioned_app();
    let mut client_app = conditioned_app();

    // Handshake first, unconditioned, so authorization isn't delayed.
    server_app.connect_client(&mut client_app);

    let step = Duration::from_millis(100);
    client_app.insert_resource(TimeUpdateStrategy::ManualDuration(step));
    client_app.insert_resource(GlobalConditionerConfig(ConditionerConfig {
        latency: 250,
        jitter: 0,
        loss: 0.0,
        duplication: 0.0,
    }));

    server_app.world_mut().spawn(Replicated);
    server_app.update();
    server_app.exchange_with_client(&mut client_app);

    // The spawn is now buffered on the client; the first update holds it.
    client_app.update();
    assert_eq!(
        replicated_count(&mut client_app),
        0,
        "the spawn must be held while latency has not elapsed"
    );

    // Advance past the latency; the held spawn is released and applied.
    for _ in 0..4 {
        client_app.update();
    }
    assert_eq!(
        replicated_count(&mut client_app),
        1,
        "the spawn must arrive once latency has elapsed"
    );
}
