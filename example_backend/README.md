# Bevy Replicon Example Backend

A simple TCP backend for running examples, testing backend API and serving as a reference for backend implementation.

> [!WARNING]
> DO NOT USE this in a real project. Instead, choose a proper backend from [Messaging backends](../README.md#messaging-backends).

To run an [example](examples) use the following command:

```bash
cargo run -p bevy_replicon_example_backend --example <example name> -- <example CLI args>
```

Pass `--help` after the `--` to see the available example CLI options.

In all examples, you need to start the server first since connecting via TCP in the Rust standard library is blocking.
You won't have this issue with a real backend.
