use std::{
    io,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    time::Instant,
};

use bevy::prelude::*;
use bevy_replicon::{prelude::*, shared::backend::connected_client::NetworkId};

use super::{
    link_conditioner::{ConditionerConfig, GlobalConditionerConfig, LinkConditioner},
    tcp,
};

/// Adds a server messaging backend made for examples to `bevy_replicon`.
pub struct RepliconExampleServerPlugin;

impl Plugin for RepliconExampleServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                (
                    receive_packets.run_if(resource_exists::<ExampleServer>),
                    // Run after since the resource might be removed after receiving packets.
                    set_stopped.run_if(resource_removed::<ExampleServer>),
                )
                    .chain(),
                set_running.run_if(resource_added::<ExampleServer>),
            )
                .in_set(ServerSystems::ReceivePackets),
        )
        .add_systems(
            PostUpdate,
            send_packets
                .run_if(resource_exists::<ExampleServer>)
                .in_set(ServerSystems::SendPackets),
        );
    }
}

fn set_running(mut state: ResMut<NextState<ServerState>>) {
    state.set(ServerState::Running);
}

fn set_stopped(mut state: ResMut<NextState<ServerState>>) {
    state.set(ServerState::Stopped);
}

fn receive_packets(
    mut commands: Commands,
    mut messages: ResMut<ServerMessages>,
    server: Res<ExampleServer>,
    mut clients: Query<(Entity, &mut ExampleConnection, Option<&ConditionerConfig>)>,
    global_config: Option<Res<GlobalConditionerConfig>>,
) {
    loop {
        match server.0.accept() {
            Ok((stream, addr)) => {
                if let Err(e) = stream.set_nodelay(true) {
                    error!("unable to disable buffering for `{addr}`: {e}");
                    continue;
                }
                if let Err(e) = stream.set_nonblocking(true) {
                    error!("unable to enable non-blocking for `{addr}`: {e}");
                    continue;
                }
                let network_id = NetworkId::new(addr.port().into());
                let client = commands
                    .spawn((
                        ConnectedClient { max_size: 1200 },
                        network_id,
                        ExampleConnection {
                            stream,
                            conditioner: Default::default(),
                        },
                    ))
                    .id();
                debug!("connecting client `{client}` with `{network_id:?}`");
            }
            Err(e) => {
                if e.kind() != io::ErrorKind::WouldBlock {
                    error!("stopping server due to network error: {e}");
                    commands.remove_resource::<ExampleServer>();
                }
                break;
            }
        }
    }

    let now = Instant::now();
    for (client, mut connection, config) in &mut clients {
        let config = config.or(global_config.as_deref().map(|c| &**c));
        loop {
            match tcp::read_message(&mut connection.stream) {
                Ok((channel_id, message)) => connection
                    .conditioner
                    .insert(config, now, channel_id, message),
                Err(e) => {
                    match e.kind() {
                        io::ErrorKind::WouldBlock => (),
                        io::ErrorKind::UnexpectedEof => {
                            commands.entity(client).despawn();
                            debug!("`client {client}` closed the connection");
                        }
                        _ => {
                            commands.entity(client).despawn();
                            error!(
                                "disconnecting client `{client}` due to message read error: {e}"
                            );
                        }
                    }
                    break;
                }
            }
        }

        while let Some((channel_id, message)) = connection.conditioner.pop(now) {
            messages.insert_received(client, channel_id, message)
        }
    }
}

fn send_packets(
    mut commands: Commands,
    mut disconnects: MessageReader<DisconnectRequest>,
    mut messages: ResMut<ServerMessages>,
    mut clients: Query<&mut ExampleConnection>,
) {
    for (client, channel_id, message) in messages.drain_sent() {
        let mut connection = clients
            .get_mut(client)
            .expect("all connected clients should have streams");
        if let Err(e) = tcp::send_message(&mut connection.stream, channel_id, &message) {
            commands.entity(client).despawn();
            error!("disconnecting client `{client}` due to error: {e}");
        }
    }

    for disconnect in disconnects.read() {
        debug!("disconnecting client `{}` by request", disconnect.client);
        commands.entity(disconnect.client).despawn();
    }
}

/// The socket used by the server.
#[derive(Resource)]
pub struct ExampleServer(TcpListener);

impl ExampleServer {
    /// Opens an example server socket on the specified port.
    pub fn new(port: u16) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        listener.set_nonblocking(true)?;
        Ok(Self(listener))
    }

    /// Returns local address if the server is running.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }
}

/// A connected for a client.
#[derive(Component)]
struct ExampleConnection {
    stream: TcpStream,
    conditioner: LinkConditioner,
}
