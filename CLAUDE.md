# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

- **Build:** `cargo build`
- **Run:** `cargo run -p compose`
- **Run daemon:** `cargo run -p compose -- --daemon`
- **Test:** `cargo test`
- **Single test:** `cargo test -p compose <test_name>`
- **Lint:** `cargo clippy`
- **Format:** `cargo fmt`

## Project Overview

Cargo workspace providing reusable `ecsdk_*` crates for building async ECS applications with Bevy ECS and Tokio. The `examples/compose` binary demonstrates the framework as an ECS-driven container orchestration demo.

### Workspace Structure

- **`crates/ecsdk_core`** — Foundational types: `WorldCallback`, `CmdQueue`, `MessageQueue<M>`, signals, `AppExit`, `ScheduleControl`, `ApplyMessage` trait
- **`crates/ecsdk_app`** — Application bootstrap: `Receivers<M>`, `setup<M>()`, `run_async<M>()`
- **`crates/ecsdk_tasks`** — Entity-scoped async tasks: `AsyncTask`, `TaskQueue<M>`, `SpawnTask<M>`, `CmdOnly`, `SpawnCmdTask`
- **`crates/ecsdk_term`** — Terminal integration: `TerminalSize`, `TerminalEvent`, `TerminalGuard`, `Rect`, `TermPlugin`
- **`crates/ecsdk_replicon`** — Replicon transport: `RepliconPacket`, `run_bridge()`, `ServerBridge`, `ClientBridge`, transport plugins
- **`examples/compose`** — Container orchestration demo using all ecsdk crates

## Rust Conventions

- Async fn in traits is natively supported — use `async fn` directly. Prefer generics over `dyn Trait` to avoid needing `Pin<Box<dyn Future>>`. If runtime dispatch is needed, consider the `enum_dispatch` crate.

## Feedback Loop

Any time the user expresses a preference about code style, architecture, or patterns — whether as a direct correction, a suggestion during review, or an offhand remark — immediately record it in the auto-memory file `memory/antipatterns.md`. This includes "you should do X instead" comments even when given mid-task. Review that file before starting any task.

## Key Dependencies

- **bevy_ecs 0.18** — Entity Component System (with `multi_threaded` feature)
- **tokio** — Async runtime (with `full` features)
- **Rust edition 2024**
