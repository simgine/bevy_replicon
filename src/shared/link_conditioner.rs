//! Artificial network conditions for testing and debugging.
//!
//! [`LinkConditionerPlugin`] adds latency, jitter, packet loss and duplication to Replicon's
//! message exchange. It operates on [`ClientMessages`] and [`ServerMessages`] rather than on a
//! socket.
//!
//! The conditioning is reliability-aware so Replicon only ever sees conditions within its design:
//! latency and jitter delay every channel, but loss and duplication apply only to
//! [`Channel::Unreliable`], and [`Channel::Ordered`] delivery is never reordered. This mirrors what
//! a real transport exposes to the application - reliable channels retransmit (so the application
//! observes added delay, never loss), while unreliable ones can drop or arrive doubled.
//!
//! The plugin is opt-in and not part of [`RepliconPlugins`](crate::prelude::RepliconPlugins). Add it
//! on the client, the server, or both; on a peer it conditions traffic in both directions.

use alloc::{collections::BTreeMap, vec::Vec};
use core::time::Duration;

use bevy::prelude::*;
#[cfg(any(feature = "client", feature = "server"))]
use bytes::Bytes;

use crate::shared::backend::channels::Channel;
#[cfg(any(feature = "client", feature = "server"))]
use crate::shared::backend::channels::RepliconChannels;
#[cfg(feature = "client")]
use crate::{client::ClientSystems, shared::backend::client_messages::ClientMessages};
#[cfg(feature = "server")]
use crate::{server::ServerSystems, shared::backend::server_messages::ServerMessages};

/// Adds artificial network conditions to Replicon's message exchange.
///
/// Inserts the [`LinkConditioner`] resource (default conditions are a no-op) and the systems that
/// hold and release messages around the backend's receive and send. See the [module
/// documentation](self) for the behavior and guarantees.
pub struct LinkConditionerPlugin;

impl Plugin for LinkConditionerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LinkConditioner>();

        #[cfg(feature = "client")]
        app.init_resource::<ClientConditionerState>()
            .add_systems(
                PreUpdate,
                condition_client_inbound
                    .after(ClientSystems::ReceivePackets)
                    .before(ClientSystems::Receive),
            )
            .add_systems(
                PostUpdate,
                condition_client_outbound
                    .after(ClientSystems::Send)
                    .before(ClientSystems::SendPackets),
            );

        #[cfg(feature = "server")]
        app.init_resource::<ServerConditionerState>()
            .add_systems(
                PreUpdate,
                condition_server_inbound
                    .after(ServerSystems::ReceivePackets)
                    .before(ServerSystems::Receive),
            )
            .add_systems(
                PostUpdate,
                condition_server_outbound
                    .after(ServerSystems::Send)
                    .before(ServerSystems::SendPackets),
            );
    }
}

/// Artificial network conditions applied by [`LinkConditionerPlugin`].
///
/// All fields default to zero, which is a transparent pass-through. Values apply per message and
/// per direction, so setting them on one peer for a round trip roughly doubles the perceived
/// latency.
#[derive(Resource, Reflect, Clone, Debug, Default)]
pub struct LinkConditioner {
    /// Base delay added to every message.
    pub latency: Duration,

    /// Upper bound of an extra uniform-random delay added on top of [`Self::latency`], sampled per
    /// message. Models jitter.
    pub jitter: Duration,

    /// Probability in `0.0..=1.0` to drop a message.
    ///
    /// Applies only to [`Channel::Unreliable`]; reliable channels are never dropped.
    pub loss: f32,

    /// Probability in `0.0..=1.0` to deliver an extra copy of a message.
    ///
    /// Applies only to [`Channel::Unreliable`]; the duplicate gets its own independent delay.
    pub duplication: f32,
}

impl LinkConditioner {
    /// Returns `false` when all conditions are zero, letting the systems skip idle work.
    fn enabled(&self) -> bool {
        !self.latency.is_zero()
            || !self.jitter.is_zero()
            || self.loss > 0.0
            || self.duplication > 0.0
    }

    /// Samples the delay for a single message.
    fn delay(&self, rng: &mut Rng) -> Duration {
        if self.jitter.is_zero() {
            self.latency
        } else {
            self.latency + self.jitter.mul_f32(rng.next_f32())
        }
    }
}

#[cfg(feature = "client")]
fn condition_client_inbound(
    time: Res<Time<Real>>,
    conditioner: Res<LinkConditioner>,
    channels: Res<RepliconChannels>,
    mut state: ResMut<ClientConditionerState>,
    mut messages: ResMut<ClientMessages>,
) {
    let now = time.elapsed();
    let ClientConditionerState { inbound, rng, .. } = &mut *state;
    if !conditioner.enabled() && inbound.is_empty() {
        return;
    }

    for channel_id in 0..channels.server_channels().len() {
        let channel = channels.server_channels()[channel_id];
        for message in messages.receive(channel_id) {
            inbound.push(
                now,
                channel_id,
                channel_id,
                channel,
                message,
                &conditioner,
                rng,
            );
        }
    }
    inbound.release_due(now, |channel_id, message| {
        messages.insert_received(channel_id, message);
    });
}

#[cfg(feature = "client")]
fn condition_client_outbound(
    time: Res<Time<Real>>,
    conditioner: Res<LinkConditioner>,
    channels: Res<RepliconChannels>,
    mut state: ResMut<ClientConditionerState>,
    mut messages: ResMut<ClientMessages>,
) {
    let now = time.elapsed();
    let ClientConditionerState { outbound, rng, .. } = &mut *state;
    if !conditioner.enabled() && outbound.is_empty() {
        return;
    }

    for (channel_id, message) in messages.drain_sent() {
        let channel = channels.client_channels()[channel_id];
        outbound.push(
            now,
            channel_id,
            channel_id,
            channel,
            message,
            &conditioner,
            rng,
        );
    }
    outbound.release_due(now, |channel_id, message| {
        messages.send(channel_id, message);
    });
}

#[cfg(feature = "server")]
fn condition_server_inbound(
    time: Res<Time<Real>>,
    conditioner: Res<LinkConditioner>,
    channels: Res<RepliconChannels>,
    mut state: ResMut<ServerConditionerState>,
    mut messages: ResMut<ServerMessages>,
) {
    let now = time.elapsed();
    let ServerConditionerState { inbound, rng, .. } = &mut *state;
    if !conditioner.enabled() && inbound.is_empty() {
        return;
    }

    for channel_id in 0..channels.client_channels().len() {
        let channel = channels.client_channels()[channel_id];
        for (client, message) in messages.receive(channel_id) {
            let key = (client.to_bits(), channel_id);
            inbound.push(
                now,
                key,
                channel_id,
                channel,
                (client, message),
                &conditioner,
                rng,
            );
        }
    }
    inbound.release_due(now, |channel_id, (client, message)| {
        messages.insert_received(client, channel_id, message);
    });
}

#[cfg(feature = "server")]
fn condition_server_outbound(
    time: Res<Time<Real>>,
    conditioner: Res<LinkConditioner>,
    channels: Res<RepliconChannels>,
    mut state: ResMut<ServerConditionerState>,
    mut messages: ResMut<ServerMessages>,
) {
    let now = time.elapsed();
    let ServerConditionerState { outbound, rng, .. } = &mut *state;
    if !conditioner.enabled() && outbound.is_empty() {
        return;
    }

    for (client, channel_id, message) in messages.drain_sent() {
        let channel = channels.server_channels()[channel_id];
        let key = (client.to_bits(), channel_id);
        outbound.push(
            now,
            key,
            channel_id,
            channel,
            (client, message),
            &conditioner,
            rng,
        );
    }
    outbound.release_due(now, |channel_id, (client, message)| {
        messages.send(client, channel_id, message);
    });
}

/// Per-direction delay buffers and shared RNG for the client.
#[cfg(feature = "client")]
#[derive(Resource)]
struct ClientConditionerState {
    inbound: DelayBuffer<usize, Bytes>,
    outbound: DelayBuffer<usize, Bytes>,
    rng: Rng,
}

#[cfg(feature = "client")]
impl Default for ClientConditionerState {
    fn default() -> Self {
        Self {
            inbound: DelayBuffer::default(),
            outbound: DelayBuffer::default(),
            rng: Rng::new(INITIAL_SEED),
        }
    }
}

/// Per-direction delay buffers and shared RNG for the server.
#[cfg(feature = "server")]
#[derive(Resource)]
struct ServerConditionerState {
    inbound: DelayBuffer<(u64, usize), (Entity, Bytes)>,
    outbound: DelayBuffer<(u64, usize), (Entity, Bytes)>,
    rng: Rng,
}

#[cfg(feature = "server")]
impl Default for ServerConditionerState {
    fn default() -> Self {
        Self {
            inbound: DelayBuffer::default(),
            outbound: DelayBuffer::default(),
            rng: Rng::new(INITIAL_SEED),
        }
    }
}

/// Messages held until their release time, in arrival order.
///
/// `K` is the per-channel ordering key used to keep [`Channel::Ordered`] release times monotonic
/// (the channel ID on a client, the client entity plus channel on a server).
struct DelayBuffer<K, P> {
    held: Vec<Held<P>>,
    next_ordered: BTreeMap<K, Duration>,
}

impl<K, P> Default for DelayBuffer<K, P> {
    fn default() -> Self {
        Self {
            held: Vec::new(),
            next_ordered: BTreeMap::new(),
        }
    }
}

impl<K: Ord, P: Clone> DelayBuffer<K, P> {
    fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Schedules a message for delivery, applying loss and duplication.
    fn push(
        &mut self,
        now: Duration,
        order_key: K,
        channel_id: usize,
        channel: Channel,
        payload: P,
        conditioner: &LinkConditioner,
        rng: &mut Rng,
    ) {
        if channel == Channel::Unreliable && rng.chance(conditioner.loss) {
            return;
        }

        let release = self.schedule(now, order_key, channel, conditioner, rng);
        if channel == Channel::Unreliable && rng.chance(conditioner.duplication) {
            let release = now + conditioner.delay(rng);
            self.held.push(Held {
                release,
                channel_id,
                payload: payload.clone(),
            });
        }
        self.held.push(Held {
            release,
            channel_id,
            payload,
        });
    }

    /// Returns the release time, clamped to keep ordered channels in order.
    fn schedule(
        &mut self,
        now: Duration,
        order_key: K,
        channel: Channel,
        conditioner: &LinkConditioner,
        rng: &mut Rng,
    ) -> Duration {
        let release = now + conditioner.delay(rng);
        if channel != Channel::Ordered {
            return release;
        }

        let last = self.next_ordered.entry(order_key).or_insert(Duration::ZERO);
        *last = release.max(*last);
        *last
    }

    /// Emits every message whose release time has passed, in arrival order.
    fn release_due(&mut self, now: Duration, mut emit: impl FnMut(usize, P)) {
        let mut kept = Vec::with_capacity(self.held.len());
        for held in core::mem::take(&mut self.held) {
            if held.release <= now {
                emit(held.channel_id, held.payload);
            } else {
                kept.push(held);
            }
        }
        self.held = kept;
    }
}

struct Held<P> {
    release: Duration,
    channel_id: usize,
    payload: P,
}

/// Fixed seed so conditioned runs are reproducible.
const INITIAL_SEED: u64 = 0x2545_F491_4F6C_DD1D;

/// SplitMix64, an inline PRNG so the crate stays `no_std` without a `rand` dependency.
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform `f32` in `[0, 1)`, from the top 24 bits.
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Rolls against a probability, leaving the RNG untouched when `p` is zero.
    fn chance(&mut self, p: f32) -> bool {
        p > 0.0 && self.next_f32() < p
    }
}

#[cfg(all(test, any(feature = "client", feature = "server")))]
mod tests {
    use test_log::test;

    use super::*;

    const MSG: &[u8] = &[1, 2, 3];

    fn drained(buffer: &mut DelayBuffer<usize, Bytes>, now: Duration) -> Vec<usize> {
        let mut out = Vec::new();
        buffer.release_due(now, |channel_id, _| out.push(channel_id));
        out
    }

    #[test]
    fn rng_stays_in_range() {
        let mut rng = Rng::new(INITIAL_SEED);
        for _ in 0..10_000 {
            let value = rng.next_f32();
            assert!((0.0..1.0).contains(&value));
        }
    }

    #[test]
    fn passthrough_releases_immediately() {
        let conditioner = LinkConditioner::default();
        let mut rng = Rng::new(INITIAL_SEED);
        let mut buffer = DelayBuffer::default();

        buffer.push(
            Duration::ZERO,
            0,
            0,
            Channel::Ordered,
            Bytes::from(MSG),
            &conditioner,
            &mut rng,
        );

        assert_eq!(drained(&mut buffer, Duration::ZERO), [0]);
    }

    #[test]
    fn latency_delays_release() {
        let conditioner = LinkConditioner {
            latency: Duration::from_millis(100),
            ..Default::default()
        };
        let mut rng = Rng::new(INITIAL_SEED);
        let mut buffer = DelayBuffer::default();

        buffer.push(
            Duration::ZERO,
            0,
            0,
            Channel::Ordered,
            Bytes::from(MSG),
            &conditioner,
            &mut rng,
        );

        assert!(drained(&mut buffer, Duration::from_millis(50)).is_empty());
        assert_eq!(drained(&mut buffer, Duration::from_millis(100)), [0]);
    }

    #[test]
    fn loss_drops_only_unreliable() {
        let conditioner = LinkConditioner {
            loss: 1.0,
            ..Default::default()
        };
        let mut rng = Rng::new(INITIAL_SEED);
        let mut buffer = DelayBuffer::default();

        buffer.push(
            Duration::ZERO,
            0,
            0,
            Channel::Unreliable,
            Bytes::from(MSG),
            &conditioner,
            &mut rng,
        );
        buffer.push(
            Duration::ZERO,
            1,
            1,
            Channel::Ordered,
            Bytes::from(MSG),
            &conditioner,
            &mut rng,
        );

        assert_eq!(drained(&mut buffer, Duration::ZERO), [1]);
    }

    #[test]
    fn duplication_doubles_only_unreliable() {
        let conditioner = LinkConditioner {
            duplication: 1.0,
            ..Default::default()
        };
        let mut rng = Rng::new(INITIAL_SEED);
        let mut buffer = DelayBuffer::default();

        buffer.push(
            Duration::ZERO,
            0,
            0,
            Channel::Unreliable,
            Bytes::from(MSG),
            &conditioner,
            &mut rng,
        );
        buffer.push(
            Duration::ZERO,
            1,
            1,
            Channel::Ordered,
            Bytes::from(MSG),
            &conditioner,
            &mut rng,
        );

        assert_eq!(drained(&mut buffer, Duration::ZERO), [0, 0, 1]);
    }

    #[test]
    fn ordered_release_is_monotonic() {
        let conditioner = LinkConditioner {
            jitter: Duration::from_millis(100),
            ..Default::default()
        };
        let mut rng = Rng::new(INITIAL_SEED);
        let mut buffer = DelayBuffer::<usize, Bytes>::default();

        for _ in 0..32 {
            buffer.push(
                Duration::ZERO,
                0,
                0,
                Channel::Ordered,
                Bytes::from(MSG),
                &conditioner,
                &mut rng,
            );
        }

        let mut releases: Vec<Duration> = buffer.held.iter().map(|held| held.release).collect();
        let mut sorted = releases.clone();
        sorted.sort_unstable();
        assert_eq!(releases, sorted, "ordered releases must not decrease");

        releases.dedup();
        assert!(
            releases.len() > 1,
            "jitter should still spread releases apart"
        );
    }
}
