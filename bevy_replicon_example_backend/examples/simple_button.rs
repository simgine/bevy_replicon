//! A button whose state is synchronized between the server and clients.
//! Both the server and clients can toggle its state using remote triggers,
//! and the server replicates the updated state back to all clients.

use std::net::{IpAddr, Ipv4Addr};

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_example_backend::{ExampleClient, ExampleServer, RepliconExampleBackendPlugins};
use clap::Parser;
use serde::{Deserialize, Serialize};

fn main() {
    App::new()
        .init_resource::<Cli>() // Parse CLI before creating window.
        .add_plugins((
            DefaultPlugins,
            RepliconPlugins,
            RepliconExampleBackendPlugins,
        ))
        .replicate::<UiRoot>()
        .replicate::<ToggleButton>()
        .replicate_filtered::<ChildOf, With<ToggleButton>>() // Replicate parent only for `ToggleButton`.
        .add_client_trigger::<RemoteToggle>(Channel::Unordered)
        .add_observer(init_toggle_button)
        .add_observer(trigger_remote_toggle)
        .add_observer(apply_remote_toggle)
        .add_systems(Startup, setup)
        .add_systems(Update, (update_button_background, update_toggle_text))
        .run();
}

fn setup(mut commands: Commands, cli: Res<Cli>) -> Result<()> {
    commands.spawn(Camera2d);

    match *cli {
        Cli::SinglePlayer => {
            info!("starting single-player game");
            commands.spawn((UiRoot, children![ToggleButton(false)]));
        }
        Cli::Server { port } => {
            info!("starting server at port {port}");

            // Backend initialization
            let server = ExampleServer::new(port)?;
            commands.insert_resource(server);

            commands.spawn((UiRoot, children![ToggleButton(false)]));
            commands.spawn(Text::new("Server"));
        }
        Cli::Client { ip, port } => {
            info!("connecting to {ip}:{port}");

            // Backend initialization
            let client = ExampleClient::new((ip, port))?;
            let addr = client.local_addr()?;
            commands.insert_resource(client);

            commands.spawn(Text(format!("Client: {addr}")));
        }
    }

    Ok(())
}

/// Since we can't include hierarchy into required components, initialize it on insertion.
fn init_toggle_button(add: On<Add, ToggleButton>, mut commands: Commands) {
    commands.entity(add.entity).with_child((
        Text::default(),
        TextShadow::default(),
        TextFont {
            font_size: 30.0,
            ..Default::default()
        },
    ));
}

/// Replicating all user clicks would be expensive and unnecessary,
/// so we only replicate clicks on [`ToggleButton`].
///
/// Used on both the server and the clients.
/// Triggering this on the server will emit [`FromClient`] with [`ClientId::Server`].
fn trigger_remote_toggle(
    click: On<Pointer<Click>>,
    mut commands: Commands,
    buttons: Query<(), With<ToggleButton>>,
) {
    if buttons.get(click.entity).is_ok() {
        commands.client_trigger(RemoteToggle {
            entity: click.entity,
        });
    }
}

/// Toggles the state of the button.
///
/// Executed on server and singleplayer.
/// The state will be replicated back to clients.
fn apply_remote_toggle(
    toggle: On<FromClient<RemoteToggle>>,
    mut buttons: Query<&mut ToggleButton>,
) {
    if let Ok(mut toggle) = buttons.get_mut(toggle.entity) {
        **toggle = !**toggle
    }
}

fn update_button_background(
    mut buttons: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, With<Button>)>,
) {
    for (interaction, mut background_color) in &mut buttons {
        *background_color = match interaction {
            Interaction::Pressed => Color::srgb(0.35, 0.75, 0.35).into(),
            Interaction::Hovered => Color::srgb(0.25, 0.25, 0.25).into(),
            Interaction::None => Color::srgb(0.15, 0.15, 0.15).into(),
        }
    }
}

/// Updates text on the button.
///
/// It's not a trigger because on spawn the hierarchy won't be available yet.
/// See <https://github.com/bevyengine/bevy/issues/20833> for more details.
fn update_toggle_text(
    buttons: Query<(&ToggleButton, &Children), Changed<ToggleButton>>,
    mut texts: Query<&mut Text>,
) {
    for (&toggle, children) in buttons {
        if let Some(mut text) = texts.iter_many_mut(children).fetch_next() {
            text.clear();
            if *toggle {
                text.push_str("On");
            } else {
                text.push_str("Off");
            }
        }
    }
}

const DEFAULT_PORT: u16 = 5000;

/// A simple demo with a replicated button.
#[derive(Parser, PartialEq, Resource)]
enum Cli {
    /// Play locally.
    SinglePlayer,
    /// Create a server that acts as both player and host.
    Server {
        #[arg(short, long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Connect to a host.
    Client {
        #[arg(short, long, default_value_t = Ipv4Addr::LOCALHOST.into())]
        ip: IpAddr,

        #[arg(short, long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
}

impl Default for Cli {
    fn default() -> Self {
        Self::parse()
    }
}

/// Root to which all replicated UI components are attached.
///
/// We replicate only this single component and initialize the rest using required components
/// to reduce network traffic. This works for initialization on both the server (when the entity with
/// this component is spawned) and the client (when the entity with this component is replicated).
#[derive(Component, Serialize, Deserialize)]
#[require(
    Replicated,
    Node {
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        ..Default::default()
    }
)]
struct UiRoot;

/// A button that can have on and off state.
///
/// Follows a pattern similar to [`UiRoot`]. Since we replicate [`ChildOf`],
/// it will be automatically attached to the root entity.
///
/// On replication Bevy will emit a warning that can be safely ignored.
/// See <https://github.com/bevyengine/bevy/issues/19776> for more details.
#[derive(Component, Serialize, Deserialize, Deref, DerefMut, Clone, Copy)]
#[require(
    Replicated,
    Button,
    Node {
        width: Val::Px(150.0),
        height: Val::Px(65.0),
        border: UiRect::all(Val::Px(5.0)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..Default::default()
    },
    BorderColor::all(Color::WHITE),
    BorderRadius::MAX,
)]
struct ToggleButton(bool);

/// Toggles the buttons it targets.
#[derive(EntityEvent, Serialize, Deserialize)]
struct RemoteToggle {
    entity: Entity,
}
