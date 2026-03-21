# ecsdk

A Rust framework for building async ECS applications with [Bevy ECS](https://bevyengine.org/) and [Tokio](https://tokio.rs/).

Provides reusable `ecsdk_*` crates that handle the plumbing between async tasks and the ECS world — channel-based command queues, entity-scoped task spawning, terminal integration, network transport, and tracing.

## Crates

| Crate | Description |
|---|---|
| `ecsdk` | Meta crate that re-exports the framework crates and common prelude imports |
| `ecsdk_core` | Foundational types: `CmdQueue`, `MessageQueue<M>`, `ApplyMessage` trait, signals, `AppExit` |
| `ecsdk_app` | Application bootstrap: `setup<M>()`, `run_async<M>()` with biased select loop |
| `ecsdk_tasks` | Entity-scoped async tasks: `SpawnTask<M>`, `SpawnCmdTask`, `TaskQueue<M>` |
| `ecsdk_term` | Terminal integration: raw mode, alternate screen, resize handling, `TerminalGuard` |
| `ecsdk_network` | Networked app composition on top of Bevy Replicon: transport plugins, isomorphic app/plugin helpers, request/response patterns |
| `ecsdk_transport` | Pure async transport (no bevy dep): `RepliconPacket`, `run_bridge()` over any `AsyncRead + AsyncWrite` |
| `ecsdk_tracing` | Tracing-to-ECS bridge: captures `tracing` events and routes them into the ECS via entity-scoped spans |

## Example: `compose`

`examples/compose` is a container orchestration demo that exercises the full framework. It simulates pulling images, booting containers with dependency ordering, and graceful shutdown — all rendered in a live TUI.

```
cargo run -p compose           # launches daemon + TUI client
cargo run -p compose -- --daemon  # daemon only (headless)
```

The daemon and client communicate over a Unix socket using Bevy Replicon for state replication.

## Building

```
cargo build       # all crates
cargo test        # all tests
cargo clippy      # lint
cargo fmt         # format
```

## Architecture

The core loop (`ecsdk_app::run_async`) is a `tokio::select!` that drains:
- **State events** (`MessageQueue<M>`) — domain messages applied to the world
- **Command callbacks** (`CmdQueue`) — arbitrary `FnOnce(&mut World)` closures from async tasks
- **Wake/tick signals** — immediate schedule runs vs FPS-bounded rendering

Async tasks spawned with `SpawnTask` / `SpawnCmdTask` get a `TaskQueue` handle that provides `send()` (world mutation), `send_state()` (domain events), and `wake()` (schedule trigger). Entity identity is preserved across the async boundary.

## Key Dependencies

- Bevy ECS 0.18
- Tokio (full)
- Rust edition 2024
