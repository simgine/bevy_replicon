use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    time::{Duration, Instant},
};

use bevy::prelude::*;
use bevy_replicon::bytes::Bytes;
use fastrand::Rng;

/// Adds [`ConditionerConfig`] to [`AppTypeRegistry`].
pub struct LinkConditionerPlugin;

impl Plugin for LinkConditionerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<ConditionerConfig>();
    }
}

#[derive(Default)]
pub(super) struct LinkConditioner {
    rng: Rng,
    heap: BinaryHeap<TimedMessage>,
}

impl LinkConditioner {
    pub(super) fn insert(
        &mut self,
        config: Option<&ConditionerConfig>,
        mut timestamp: Instant,
        channel_id: u8,
        message: Bytes,
    ) {
        if let Some(config) = config {
            if self.rng.f32() <= config.loss {
                trace!("simulating a message drop for channel {channel_id}");
                return;
            }

            let mut latency = config.latency;
            if config.jitter > 0 {
                let jitter = self.rng.u16(0..config.jitter);
                if self.rng.bool() {
                    latency += jitter;
                } else {
                    latency = latency.saturating_sub(jitter);
                }
            }

            trace!("simulating {latency} ms latency for channel {channel_id}");
            timestamp += Duration::from_millis(latency.into());
        }

        self.heap.push(TimedMessage {
            timestamp,
            channel_id,
            message,
        });
    }

    pub(super) fn pop(&mut self, now: Instant) -> Option<(u8, Bytes)> {
        if self.heap.peek().is_some_and(|m| now >= m.timestamp) {
            // Item's instant has passed, so it's ready to be returned.
            self.heap.pop().map(|m| (m.channel_id, m.message))
        } else {
            None
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
struct TimedMessage {
    timestamp: Instant,
    channel_id: u8,
    message: Bytes,
}

impl Ord for TimedMessage {
    fn cmp(&self, other: &TimedMessage) -> Ordering {
        other.timestamp.cmp(&self.timestamp)
    }
}

impl PartialOrd for TimedMessage {
    fn partial_cmp(&self, other: &TimedMessage) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Configuration for simulating various network conditions.
///
/// When inserted as a resource, these settings apply to all received messages:
/// - For a client, it affects messages received from the server.
/// - For a server, it affects messages received from all connected clients.
///
/// You can also insert this as a component on a connected entity.
/// This will affect only that specific entity and take priority over
/// the resource configuration.
#[derive(Resource, Component, Debug, Clone, Copy, Reflect)]
pub struct ConditionerConfig {
    /// Base delay for incoming messages in milliseconds.
    pub latency: u16,

    /// Maximum additional random latency for incoming messages in milliseconds.
    ///
    /// This value is either added to **or** subtracted from [`Self::latency`].
    pub jitter: u16,

    /// The probability of an incoming packet being dropped.
    ///
    /// Represented as a value between 0 and 1.
    pub loss: f32,
}

impl ConditionerConfig {
    pub const VERY_GOOD: Self = Self {
        latency: 12,
        jitter: 3,
        loss: 0.001,
    };

    pub const GOOD: Self = Self {
        latency: 40,
        jitter: 10,
        loss: 0.002,
    };

    pub const AVERAGE: Self = Self {
        latency: 100,
        jitter: 25,
        loss: 0.02,
    };

    pub const POOR: Self = Self {
        latency: 200,
        jitter: 50,
        loss: 0.04,
    };

    pub const VERY_POOR: Self = Self {
        latency: 300,
        jitter: 75,
        loss: 0.06,
    };
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test]
    fn latency() {
        let config = ConditionerConfig {
            latency: 300,
            jitter: 0,
            loss: 0.0,
        };

        let now = Instant::now();
        let mut conditioner = LinkConditioner::default();
        conditioner.rng.seed(0);
        conditioner.insert(Some(&config), now, 0, Bytes::new());

        assert!(conditioner.pop(now).is_none());

        let passed = now + Duration::from_millis(config.latency.into());
        assert!(conditioner.pop(passed).is_some());
    }

    #[test]
    fn jitter() {
        let config = ConditionerConfig {
            latency: 0,
            jitter: 300,
            loss: 0.0,
        };

        let now = Instant::now();
        let mut conditioner = LinkConditioner::default();
        conditioner.rng.seed(0);
        conditioner.insert(Some(&config), now, 0, Bytes::new());

        assert!(conditioner.pop(now).is_none());

        let passed = now + Duration::from_millis(config.jitter.into());
        assert!(conditioner.pop(passed).is_some());
    }

    #[test]
    fn loss() {
        let config = ConditionerConfig {
            latency: 0,
            jitter: 0,
            loss: 1.0,
        };

        let mut conditioner = LinkConditioner::default();
        conditioner.rng.seed(0);
        conditioner.insert(Some(&config), Instant::now(), 0, Bytes::new());

        assert!(conditioner.heap.is_empty());
    }
}
