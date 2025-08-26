# Bevy Replicon

[![crates.io](https://img.shields.io/crates/v/bevy_replicon)](https://crates.io/crates/bevy_replicon)
[![docs.rs](https://docs.rs/bevy_replicon/badge.svg)](https://docs.rs/bevy_replicon)
[![license](https://img.shields.io/crates/l/bevy_replicon)](#license)
[![codecov](https://codecov.io/gh/simgine/bevy_replicon/graph/badge.svg?token=N1G28NQB1L)](https://codecov.io/gh/simgine/bevy_replicon)

A server-authoritative replication crate for [Bevy](https://bevyengine.org).

If you are new to networking, see [glossary](https://gist.github.com/maniwani/f92cc5d827b00163f5846ea7dcb90d44) and
[What kind of networking should X game use?](https://github.com/bevyengine/bevy/discussions/8675).

## Features

- Automatic world replication.
- Remote events and triggers.
- Authorization support.
- Control over client visibility of entities and events.
- Specify which entities should be replicated in sync using ECS relationships.
- Replication into scene to save server state.
- Customizable serialization and deserialization even for types that don't implement `serde` traits (like `Box<dyn Reflect>`).
- Abstracts game logic to support singleplayer, client, dedicated server, and listen server configurations simultaneously.
- No builtin I/O, can be used with any messaging library. See [messaging backends](#messaging-backends) for already available integrations.
- Heavily optimized.
- Robust, with high test coverage.
- Supports `no_std` and environments without atomic CAS, such as thumbv6m.
- Hackable. See [ecosystem](#ecosystem).

## Getting Started

Check out the [quick start guide](https://docs.rs/bevy_replicon).

For examples navigate to the [`bevy_replicon_example_backend`](bevy_replicon_example_backend) (because you need I/O in order to run them).
[Messaging backend](#messaging-backends) repositories usually also contains the exact same examples, but with backend initialization adapted to each backend's API.

You can also:
- Watch [my talk at Bevy Meetup #9](https://www.youtube.com/watch?v=aDsVFmXD2cc)
- Read [this great article](https://www.hankruiger.com/posts/adding-networked-multiplayer-to-my-game-with-bevy-replicon) *(not mine)*

Have any questions? Feel free to ask in the dedicated [`bevy_replicon` channel](https://discord.com/channels/691052431525675048/1090432346907492443) in Bevy's Discord server.

## Ecosystem

Depending on your game, you may need additional features. We provide an API that allows external crates to integrate with Replicon or to be built on top of it.

> [!WARNING]
> Ensure that your `bevy_replicon` version is compatible with the used crate according to semantic versioning.

#### Messaging backends

- [`bevy_replicon_renet2`](https://github.com/UkoeHB/renet2/tree/main/bevy_replicon_renet2) - integration for [`bevy_renet2`](https://github.com/UkoeHB/renet2/tree/main/bevy_renet2).
- [`bevy_replicon_quinnet`](https://github.com/Henauxg/bevy_replicon_quinnet) - integration for [`bevy_quinnet`](https://github.com/Henauxg/bevy_quinnet).
- [`aeronet_replicon`](https://github.com/aecsocket/aeronet/tree/main/crates/aeronet_replicon) - integration for [`aeronet`](https://github.com/aecsocket/aeronet).
- [`bevy_replicon_matchbox`](https://github.com/Occuros/bevy_replicon_matchbox) - integration for [`matchbox`](https://github.com/johanhelsing/matchbox).
- [`bevy_replicon_renet`](https://github.com/simgine/bevy_replicon_renet) - integration for [`bevy_renet`](https://github.com/lucaspoffo/renet/tree/master/bevy_renet).

#### Interpolation and/or rollback

- [`bevy_rewind`](https://github.com/NiseVoid/bevy_rewind) - server-authoritative rollback inspired by Rocket League.

#### Visibility

- [`bevy_replicon_attributes`](https://github.com/UkoeHB/bevy_replicon_attributes) - ergonomic visibility control through client attributes and entity/event visibility conditions. An extension of this crate's raw client visibility API.

#### Miscellaneous

- [`bevy_bundlication`](https://github.com/NiseVoid/bevy_bundlication) - automates the creation of replication rules for bundles.

#### Unmaintained

- [`bevy_replicon_repair`](https://github.com/UkoeHB/bevy_replicon_repair) - preserves replicated client state across reconnects.
- [`bevy_replicon_snap`](https://github.com/Bendzae/bevy_replicon_snap) - snapshot interpolation and client-side prediction.
- [`bevy_timewarp`](https://github.com/RJ/bevy_timewarp) - a rollback library that buffers component state. See [this instruction](https://github.com/RJ/bevy_timewarp/blob/main/REPLICON_INTEGRATION.md) about how to integrate.

## Bevy compatibility

| bevy   | bevy_replicon |
| ------ | ------------- |
| 0.16.0 | 0.33-0.34     |
| 0.15.0 | 0.29-0.32     |
| 0.14.0 | 0.27-0.28     |
| 0.13.0 | 0.23-0.26     |
| 0.12.1 | 0.18-0.22     |
| 0.11.0 | 0.6-0.17      |
| 0.10.1 | 0.2-0.6       |
| 0.10.0 | 0.1           |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
