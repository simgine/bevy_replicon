use std::{
    io,
    net::{SocketAddr, TcpStream},
    time::Instant,
};

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use super::{
    link_conditioner::{ConditionerConfig, LinkConditioner},
    tcp,
};

/// Adds a client messaging backend made for examples to `bevy_replicon`.
pub struct RepliconExampleClientPlugin;

impl Plugin for RepliconExampleClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                set_connected.run_if(resource_added::<ExampleClient>),
                receive_packets.run_if(resource_exists::<ExampleClient>),
            )
                .chain()
                .in_set(ClientSet::ReceivePackets),
        )
        .add_systems(
            PostUpdate,
            (
                set_disconnected
                    .in_set(ClientSet::PrepareSend)
                    .run_if(resource_removed::<ExampleClient>),
                send_packets
                    .in_set(ClientSet::SendPackets)
                    .run_if(resource_exists::<ExampleClient>),
            ),
        );
    }
}

fn set_disconnected(mut replicon_client: ResMut<RepliconClient>) {
    replicon_client.set_status(RepliconClientStatus::Disconnected);
}

fn set_connected(mut replicon_client: ResMut<RepliconClient>) {
    replicon_client.set_status(RepliconClientStatus::Connected);
}

fn receive_packets(
    mut commands: Commands,
    mut client: ResMut<ExampleClient>,
    mut replicon_client: ResMut<RepliconClient>,
    config: Option<Res<ConditionerConfig>>,
) {
    let now = Instant::now();
    let config = config.as_deref();
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
        replicon_client.insert_received(channel_id, message);
    }
}

fn send_packets(
    mut commands: Commands,
    mut client: ResMut<ExampleClient>,
    mut replicon_client: ResMut<RepliconClient>,
) {
    for (channel_id, message) in replicon_client.drain_sent() {
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
