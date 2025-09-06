use bevy::diagnostic::DiagnosticPath;
use bevy::{
    diagnostic::{Diagnostic, Diagnostics, RegisterDiagnostic},
    prelude::*,
};

use crate::prelude::*;

/// Plugin to write [`Diagnostics`] based on [`ClientReplicationStats`] every second.
///
/// Adds [`ClientReplicationStats`] resource.
pub struct ClientDiagnosticsPlugin;

impl Plugin for ClientDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientReplicationStats>()
            .add_systems(
                PreUpdate,
                add_measurements
                    .in_set(ClientSet::Diagnostics)
                    .run_if(in_state(ClientState::Connected)),
            )
            .add_systems(
                OnEnter(ClientState::Connected),
                add_measurements.in_set(ClientSet::Diagnostics),
            )
            .register_diagnostic(
                Diagnostic::new(RTT)
                    .with_suffix(" s")
                    .with_max_history_length(DIAGNOSTIC_HISTORY_LEN),
            )
            .register_diagnostic(
                Diagnostic::new(PACKET_LOSS)
                    .with_suffix(" %")
                    .with_max_history_length(DIAGNOSTIC_HISTORY_LEN),
            )
            .register_diagnostic(
                Diagnostic::new(SENT_BPS)
                    .with_suffix(" byte/s")
                    .with_max_history_length(DIAGNOSTIC_HISTORY_LEN),
            )
            .register_diagnostic(
                Diagnostic::new(RECEIVED_BPS)
                    .with_suffix(" byte/s")
                    .with_max_history_length(DIAGNOSTIC_HISTORY_LEN),
            )
            .register_diagnostic(
                Diagnostic::new(ENTITIES_CHANGED)
                    .with_suffix(" entities changed")
                    .with_max_history_length(DIAGNOSTIC_HISTORY_LEN),
            )
            .register_diagnostic(
                Diagnostic::new(COMPONENTS_CHANGED)
                    .with_suffix(" components changed")
                    .with_max_history_length(DIAGNOSTIC_HISTORY_LEN),
            )
            .register_diagnostic(
                Diagnostic::new(MAPPINGS)
                    .with_suffix(" mappings")
                    .with_max_history_length(DIAGNOSTIC_HISTORY_LEN),
            )
            .register_diagnostic(
                Diagnostic::new(DESPAWNS)
                    .with_suffix(" despawns")
                    .with_max_history_length(DIAGNOSTIC_HISTORY_LEN),
            )
            .register_diagnostic(
                Diagnostic::new(REPLICATION_MESSAGES)
                    .with_suffix(" replication messages")
                    .with_max_history_length(DIAGNOSTIC_HISTORY_LEN),
            )
            .register_diagnostic(
                Diagnostic::new(REPLICATION_BYTES)
                    .with_suffix(" replication bytes")
                    .with_max_history_length(DIAGNOSTIC_HISTORY_LEN),
            );
    }
}

/// Round-trip time.
pub const RTT: DiagnosticPath = DiagnosticPath::const_new("client/rtt");
/// The percent of packet loss.
pub const PACKET_LOSS: DiagnosticPath = DiagnosticPath::const_new("client/packet_loss");
/// How many messages sent per second.
pub const SENT_BPS: DiagnosticPath = DiagnosticPath::const_new("client/sent_bps");
/// How many bytes received per second.
pub const RECEIVED_BPS: DiagnosticPath = DiagnosticPath::const_new("client/received_bps");

/// How many entities changed by replication.
pub const ENTITIES_CHANGED: DiagnosticPath =
    DiagnosticPath::const_new("client/replication/entities_changed");
/// How many components changed by replication.
pub const COMPONENTS_CHANGED: DiagnosticPath =
    DiagnosticPath::const_new("client/replication/components_changed");
/// How many client-mappings added by replication.
pub const MAPPINGS: DiagnosticPath = DiagnosticPath::const_new("client/replication/mappings");
/// How many despawns applied by replication.
pub const DESPAWNS: DiagnosticPath = DiagnosticPath::const_new("client/replication/despawns");
/// How many replication messages received.
pub const REPLICATION_MESSAGES: DiagnosticPath =
    DiagnosticPath::const_new("client/replication/messages");
/// How many replication bytes received.
pub const REPLICATION_BYTES: DiagnosticPath = DiagnosticPath::const_new("client/replication/bytes");

/// Max diagnostic history length.
pub const DIAGNOSTIC_HISTORY_LEN: usize = 60;

fn add_measurements(
    mut diagnostics: Diagnostics,
    stats: Res<ClientReplicationStats>,
    mut last_stats: Local<ClientReplicationStats>,
    client: Res<RepliconClient>,
) {
    diagnostics.add_measurement(&RTT, || client.stats().rtt);
    diagnostics.add_measurement(&PACKET_LOSS, || client.stats().packet_loss);
    diagnostics.add_measurement(&SENT_BPS, || client.stats().sent_bps);
    diagnostics.add_measurement(&RECEIVED_BPS, || client.stats().received_bps);

    diagnostics.add_measurement(&ENTITIES_CHANGED, || {
        (stats.entities_changed - last_stats.entities_changed) as f64
    });
    diagnostics.add_measurement(&COMPONENTS_CHANGED, || {
        (stats.components_changed - last_stats.components_changed) as f64
    });
    diagnostics.add_measurement(&MAPPINGS, || (stats.mappings - last_stats.mappings) as f64);
    diagnostics.add_measurement(&DESPAWNS, || (stats.despawns - last_stats.despawns) as f64);
    diagnostics.add_measurement(&REPLICATION_MESSAGES, || {
        (stats.messages - last_stats.messages) as f64
    });
    diagnostics.add_measurement(&REPLICATION_BYTES, || {
        (stats.bytes - last_stats.bytes) as f64
    });
    *last_stats = *stats;
}
