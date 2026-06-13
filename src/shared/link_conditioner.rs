//! Artificial network conditions for testing and debugging.
//!
//! [`LinkConditionerPlugin`] delays, drops and duplicates received messages to emulate latency,
//! jitter and packet loss. It operates on [`ClientMessages`] and [`ServerMessages`] rather than on
//! a socket, so it applies to any messaging backend (including the in-process exchange used in
//! tests, where it produces deterministic results).
//!
//! Configure it by inserting [`GlobalConditionerConfig`] to affect every connection, or a
//! [`ConditionerConfig`] component on a connected client entity to affect just that client (the
//! component takes priority). Without either, the plugin does nothing.
//!
//! The conditioning is reliability-aware so Replicon only sees conditions within its design:
//! latency and jitter delay every channel, but loss and duplication apply only to
//! [`Channel::Unreliable`], and [`Channel::Ordered`] delivery is never reordered. This mirrors what
//! a real transport exposes to the application - reliable channels retransmit (so the application
//! observes added delay, never loss), while unreliable ones can drop or arrive doubled.

use alloc::collections::{BTreeMap, BinaryHeap};
use core::{cmp::Ordering, time::Duration};

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

/// Applies artificial network conditions to received messages.
///
/// Holds and releases messages between the backend's receive and Replicon's reading. Inactive
/// until a [`GlobalConditionerConfig`] resource or a [`ConditionerConfig`] component is present.
/// See the [module documentation](self) for behavior and guarantees.
pub struct LinkConditionerPlugin;

impl Plugin for LinkConditionerPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "client")]
        app.init_resource::<ClientConditioner>().add_systems(
            PreUpdate,
            condition_client_inbound
                .after(ClientSystems::ReceivePackets)
                .before(ClientSystems::Receive),
        );

        #[cfg(feature = "server")]
        app.init_resource::<ServerConditioner>().add_systems(
            PreUpdate,
            condition_server_inbound
                .after(ServerSystems::ReceivePackets)
                .before(ServerSystems::Receive),
        );

        #[cfg(not(any(feature = "client", feature = "server")))]
        let _ = app;
    }
}

/// Network conditions for a single connection's received messages.
///
/// Insert as a component on a connected client entity to condition that client on the server, or
/// wrap in [`GlobalConditionerConfig`] to condition every connection. A per-client component takes
/// priority over the resource.
#[derive(Component, Reflect, Debug, Clone, Copy)]
pub struct ConditionerConfig {
    /// Base delay added to every message, in milliseconds.
    pub latency: u16,

    /// Maximum random latency added to **or** subtracted from [`Self::latency`], in milliseconds.
    pub jitter: u16,

    /// Probability in `0.0..=1.0` to drop a message.
    ///
    /// Applies only to [`Channel::Unreliable`]; reliable channels are never dropped.
    pub loss: f32,

    /// Probability in `0.0..=1.0` to deliver an extra copy of a message.
    ///
    /// Applies only to [`Channel::Unreliable`]; the duplicate gets its own independent delay.
    pub duplication: f32,
}

impl ConditionerConfig {
    /// A near-perfect connection.
    pub const VERY_GOOD: Self = Self {
        latency: 12,
        jitter: 3,
        loss: 0.001,
        duplication: 0.0,
    };

    /// A good connection.
    pub const GOOD: Self = Self {
        latency: 40,
        jitter: 10,
        loss: 0.002,
        duplication: 0.0,
    };

    /// An average connection.
    pub const AVERAGE: Self = Self {
        latency: 100,
        jitter: 25,
        loss: 0.02,
        duplication: 0.0,
    };

    /// A poor connection.
    pub const POOR: Self = Self {
        latency: 200,
        jitter: 50,
        loss: 0.04,
        duplication: 0.0,
    };

    /// A very poor connection.
    pub const VERY_POOR: Self = Self {
        latency: 300,
        jitter: 75,
        loss: 0.06,
        duplication: 0.0,
    };
}

/// Network conditions applied to every connection.
///
/// A per-client [`ConditionerConfig`] component takes priority over this resource.
#[derive(Resource, Deref, DerefMut, Reflect, Debug, Clone, Copy)]
pub struct GlobalConditionerConfig(pub ConditionerConfig);

#[cfg(feature = "client")]
fn condition_client_inbound(
    time: Res<Time<Real>>,
    global: Option<Res<GlobalConditionerConfig>>,
    channels: Res<RepliconChannels>,
    mut conditioner: ResMut<ClientConditioner>,
    mut messages: ResMut<ClientMessages>,
) {
    let config = global.map(|global| global.0);
    if config.is_none() && conditioner.buffer.is_empty() {
        return;
    }

    let now = time.elapsed();
    let ClientConditioner { buffer, rng } = &mut *conditioner;
    for channel_id in 0..channels.server_channels().len() {
        let channel = channels.server_channels()[channel_id];
        for message in messages.receive(channel_id) {
            buffer.accept(
                now,
                channel_id,
                channel_id,
                channel,
                message,
                config.as_ref(),
                rng,
            );
        }
    }
    buffer.release(now, |channel_id, message| {
        messages.insert_received(channel_id, message);
    });
}

#[cfg(feature = "server")]
fn condition_server_inbound(
    time: Res<Time<Real>>,
    global: Option<Res<GlobalConditionerConfig>>,
    configs: Query<&ConditionerConfig>,
    channels: Res<RepliconChannels>,
    mut conditioner: ResMut<ServerConditioner>,
    mut messages: ResMut<ServerMessages>,
) {
    let global = global.map(|global| global.0);
    if global.is_none() && configs.is_empty() && conditioner.buffer.is_empty() {
        return;
    }

    let now = time.elapsed();
    let ServerConditioner { buffer, rng } = &mut *conditioner;
    for channel_id in 0..channels.client_channels().len() {
        let channel = channels.client_channels()[channel_id];
        for (client, message) in messages.receive(channel_id) {
            let config = configs.get(client).ok().copied().or(global);
            let key = (client.to_bits(), channel_id);
            buffer.accept(
                now,
                key,
                channel_id,
                channel,
                (client, message),
                config.as_ref(),
                rng,
            );
        }
    }
    buffer.release(now, |channel_id, (client, message)| {
        messages.insert_received(client, channel_id, message);
    });
}

/// Delay buffer and RNG for the client's single connection.
#[cfg(feature = "client")]
#[derive(Resource, Default)]
struct ClientConditioner {
    buffer: ConditionerBuffer<usize, Bytes>,
    rng: Rng,
}

/// Delay buffer and RNG shared across all server connections.
#[cfg(feature = "server")]
#[derive(Resource, Default)]
struct ServerConditioner {
    buffer: ConditionerBuffer<(u64, usize), (Entity, Bytes)>,
    rng: Rng,
}

/// Messages waiting for their release time, ordered earliest-first.
///
/// `K` is the per-channel ordering key (the channel ID on a client, the client entity plus channel
/// on a server). It keeps [`Channel::Ordered`] release times monotonic so ordered delivery is
/// never reordered.
struct ConditionerBuffer<K, P> {
    queue: BinaryHeap<Pending<P>>,
    next_seq: u64,
    next_ordered: BTreeMap<K, Duration>,
}

impl<K, P> Default for ConditionerBuffer<K, P> {
    fn default() -> Self {
        Self {
            queue: BinaryHeap::new(),
            next_seq: 0,
            next_ordered: BTreeMap::new(),
        }
    }
}

impl<K: Ord, P: Clone> ConditionerBuffer<K, P> {
    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Schedules a received message, applying loss, latency, jitter and duplication.
    ///
    /// Without a config the message is released immediately, preserving order.
    fn accept(
        &mut self,
        now: Duration,
        order_key: K,
        channel_id: usize,
        channel: Channel,
        payload: P,
        config: Option<&ConditionerConfig>,
        rng: &mut Rng,
    ) {
        let Some(config) = config else {
            self.enqueue(now, channel_id, payload);
            return;
        };

        if channel == Channel::Unreliable && rng.chance(config.loss) {
            return;
        }

        let release = self.schedule(now, order_key, channel, config, rng);
        if channel == Channel::Unreliable && rng.chance(config.duplication) {
            let duplicate = now + sample_delay(config, rng);
            self.enqueue(duplicate, channel_id, payload.clone());
        }
        self.enqueue(release, channel_id, payload);
    }

    /// Returns the release time, clamped to keep ordered channels in order.
    fn schedule(
        &mut self,
        now: Duration,
        order_key: K,
        channel: Channel,
        config: &ConditionerConfig,
        rng: &mut Rng,
    ) -> Duration {
        let release = now + sample_delay(config, rng);
        if channel != Channel::Ordered {
            return release;
        }

        let last = self.next_ordered.entry(order_key).or_insert(Duration::ZERO);
        *last = release.max(*last);
        *last
    }

    fn enqueue(&mut self, release: Duration, channel_id: usize, payload: P) {
        self.queue.push(Pending {
            release,
            seq: self.next_seq,
            channel_id,
            payload,
        });
        self.next_seq += 1;
    }

    /// Emits every message whose release time has passed, earliest first.
    fn release(&mut self, now: Duration, mut emit: impl FnMut(usize, P)) {
        while self
            .queue
            .peek()
            .is_some_and(|pending| pending.release <= now)
        {
            let pending = self.queue.pop().expect("peek confirmed an item");
            emit(pending.channel_id, pending.payload);
        }
    }
}

/// A message ordered by release time, then arrival, for the min-heap.
struct Pending<P> {
    release: Duration,
    seq: u64,
    channel_id: usize,
    payload: P,
}

impl<P> PartialEq for Pending<P> {
    fn eq(&self, other: &Self) -> bool {
        self.seq == other.seq
    }
}

impl<P> Eq for Pending<P> {}

impl<P> Ord for Pending<P> {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed so the `BinaryHeap` max-heap yields the earliest release first.
        other
            .release
            .cmp(&self.release)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

impl<P> PartialOrd for Pending<P> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Samples the delay for one message: base latency with jitter added or subtracted.
fn sample_delay(config: &ConditionerConfig, rng: &mut Rng) -> Duration {
    let mut millis = i64::from(config.latency);
    if config.jitter > 0 {
        let jitter = i64::from(rng.below(config.jitter));
        millis += if rng.coin_flip() { jitter } else { -jitter };
    }
    Duration::from_millis(millis.max(0) as u64)
}

/// Fixed seed so conditioned runs are reproducible.
const INITIAL_SEED: u64 = 0x2545_F491_4F6C_DD1D;

/// SplitMix64, an inline PRNG so the crate stays `no_std` without a `rand` dependency.
struct Rng {
    state: u64,
}

impl Default for Rng {
    fn default() -> Self {
        Self::new(INITIAL_SEED)
    }
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

    fn coin_flip(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    /// Uniform value in `0..max`, or zero when `max` is zero.
    fn below(&mut self, max: u16) -> u16 {
        if max == 0 {
            0
        } else {
            (self.next_u64() % u64::from(max)) as u16
        }
    }
}

#[cfg(all(test, any(feature = "client", feature = "server")))]
mod tests {
    use test_log::test;

    use super::*;

    fn buffer() -> ConditionerBuffer<usize, Bytes> {
        ConditionerBuffer::default()
    }

    /// Drains every released message at `now`, returning each payload's first byte in pop order.
    fn drain(buffer: &mut ConditionerBuffer<usize, Bytes>, now: Duration) -> Vec<u8> {
        let mut out = Vec::new();
        buffer.release(now, |_, payload| out.push(payload[0]));
        out
    }

    fn message(tag: u8) -> Bytes {
        Bytes::copy_from_slice(&[tag])
    }

    #[test]
    fn rng_stays_in_range() {
        let mut rng = Rng::default();
        for _ in 0..10_000 {
            assert!((0.0..1.0).contains(&rng.next_f32()));
        }
    }

    #[test]
    fn without_config_releases_immediately() {
        let mut rng = Rng::default();
        let mut buffer = buffer();

        buffer.accept(
            Duration::ZERO,
            0,
            0,
            Channel::Ordered,
            message(7),
            None,
            &mut rng,
        );

        assert_eq!(drain(&mut buffer, Duration::ZERO), [7]);
    }

    #[test]
    fn latency_delays_release() {
        let config = ConditionerConfig {
            latency: 100,
            jitter: 0,
            loss: 0.0,
            duplication: 0.0,
        };
        let mut rng = Rng::default();
        let mut buffer = buffer();

        buffer.accept(
            Duration::ZERO,
            0,
            0,
            Channel::Ordered,
            message(1),
            Some(&config),
            &mut rng,
        );

        assert!(drain(&mut buffer, Duration::from_millis(50)).is_empty());
        assert_eq!(drain(&mut buffer, Duration::from_millis(100)), [1]);
    }

    #[test]
    fn loss_drops_only_unreliable() {
        let config = ConditionerConfig {
            latency: 0,
            jitter: 0,
            loss: 1.0,
            duplication: 0.0,
        };
        let mut rng = Rng::default();
        let mut buffer = buffer();

        buffer.accept(
            Duration::ZERO,
            0,
            0,
            Channel::Unreliable,
            message(1),
            Some(&config),
            &mut rng,
        );
        buffer.accept(
            Duration::ZERO,
            1,
            1,
            Channel::Ordered,
            message(2),
            Some(&config),
            &mut rng,
        );

        assert_eq!(drain(&mut buffer, Duration::ZERO), [2]);
    }

    #[test]
    fn duplication_doubles_only_unreliable() {
        let config = ConditionerConfig {
            latency: 0,
            jitter: 0,
            loss: 0.0,
            duplication: 1.0,
        };
        let mut rng = Rng::default();
        let mut buffer = buffer();

        buffer.accept(
            Duration::ZERO,
            0,
            0,
            Channel::Unreliable,
            message(1),
            Some(&config),
            &mut rng,
        );
        buffer.accept(
            Duration::ZERO,
            1,
            1,
            Channel::Ordered,
            message(2),
            Some(&config),
            &mut rng,
        );

        let drained = drain(&mut buffer, Duration::from_secs(1));
        assert_eq!(drained.iter().filter(|&&tag| tag == 1).count(), 2);
        assert_eq!(drained.iter().filter(|&&tag| tag == 2).count(), 1);
    }

    #[test]
    fn ordered_keeps_arrival_order_under_jitter() {
        let config = ConditionerConfig {
            latency: 50,
            jitter: 50,
            loss: 0.0,
            duplication: 0.0,
        };
        let mut rng = Rng::default();
        let mut buffer = buffer();

        for tag in 0..32 {
            buffer.accept(
                Duration::ZERO,
                0,
                0,
                Channel::Ordered,
                message(tag),
                Some(&config),
                &mut rng,
            );
        }

        let drained = drain(&mut buffer, Duration::from_secs(1));
        let expected: Vec<u8> = (0..32).collect();
        assert_eq!(
            drained, expected,
            "ordered channel must preserve arrival order"
        );
    }
}
