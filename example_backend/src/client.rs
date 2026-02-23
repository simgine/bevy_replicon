use std::{
    io,
    net::{SocketAddr, TcpStream},
    time::Instant,
};

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use super::{
    link_conditioner::{GlobalConditionerConfig, LinkConditioner},
    tcp,
};

/// Adds a client messaging backend made for examples to `bevy_replicon`.
pub struct RepliconExampleClientPlugin;

impl Plugin for RepliconExampleClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                (
                    receive_packets.run_if(resource_exists::<ExampleClient>),
                    // Run after since the resource might be removed after receiving packets.
                    set_disconnected.run_if(resource_removed::<ExampleClient>),
                )
                    .chain(),
                set_connected.run_if(resource_added::<ExampleClient>),
            )
                .in_set(ClientSystems::ReceivePackets),
        )
        .add_systems(
            PostUpdate,
            send_packets
                .run_if(resource_exists::<ExampleClient>)
                .in_set(ClientSystems::SendPackets),
        );
    }
}

fn set_connected(mut state: ResMut<NextState<ClientState>>) {
    state.set(ClientState::Connected);
}

fn set_disconnected(mut state: ResMut<NextState<ClientState>>) {
    state.set(ClientState::Disconnected);
}

fn receive_packets(
    mut commands: Commands,
    mut client: ResMut<ExampleClient>,
    mut messages: ResMut<ClientMessages>,
    config: Option<Res<GlobalConditionerConfig>>,
) {
    let now = Instant::now();
    let config = config.as_deref().map(|c| &**c);
    loop {
        match tcp::read_message(&mut client.stream) {
            Ok((channel_id, message)) => {
                client.conditioner.insert(config, now, channel_id, message)
            }
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock => break,
                io::ErrorKind::UnexpectedEof => {
                    debug!("server closed the connection");
                    commands.remove_resource::<ExampleClient>();
                    break;
                }
                _ => {
                    error!("disconnecting due to message read error: {e}");
                    commands.remove_resource::<ExampleClient>();
                    break;
                }
            },
        }
    }

    while let Some((channel_id, message)) = client.conditioner.pop(now) {
        messages.insert_received(channel_id, message);
    }
}

fn send_packets(
    mut commands: Commands,
    mut client: ResMut<ExampleClient>,
    mut messages: ResMut<ClientMessages>,
) {
    for (channel_id, message) in messages.drain_sent() {
        if let Err(e) = tcp::send_message(&mut client.stream, channel_id, &message) {
            error!("disconnecting due message write error: {e}");
            commands.remove_resource::<ExampleClient>();
            return;
        }
    }
}

/// The socket used by the client.
#[derive(Resource)]
pub struct ExampleClient {
    stream: TcpStream,
    conditioner: LinkConditioner,
}

impl ExampleClient {
    /// Opens an example client socket connected to a server on the specified port.
    pub fn new(addr: impl Into<SocketAddr>) -> io::Result<Self> {
        let stream = TcpStream::connect(addr.into())?;
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            conditioner: Default::default(),
        })
    }

    /// Returns local address if connected.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.stream.local_addr()
    }

    /// Returns true if the client is connected.
    pub fn is_connected(&self) -> bool {
        self.local_addr().is_ok()
    }
}
