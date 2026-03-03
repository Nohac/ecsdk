# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

- **Build:** `cargo build`
- **Run:** `cargo run`
- **Test:** `cargo test`
- **Single test:** `cargo test <test_name>`
- **Lint:** `cargo clippy`
- **Format:** `cargo fmt`

## Project Overview

Experimental project exploring Bevy ECS as a framework for managing entities and systems, driven by an async Tokio runtime. Uses Bevy's `World`, `Schedule`, and observer pattern directly (no Bevy `App`) combined with Tokio's `select!` loop and mpsc channels for event-driven updates.

## Rust Conventions

- Async fn in traits is natively supported — use `async fn` directly. Prefer generics over `dyn Trait` to avoid needing `Pin<Box<dyn Future>>`. If runtime dispatch is needed, consider the `enum_dispatch` crate.

## Feedback Loop

Any time the user expresses a preference about code style, architecture, or patterns — whether as a direct correction, a suggestion during review, or an offhand remark — immediately record it in the auto-memory file `memory/antipatterns.md`. This includes "you should do X instead" comments even when given mid-task. Review that file before starting any task.

## Key Dependencies

- **bevy_ecs 0.18** — Entity Component System (with `multi_threaded` feature)
- **tokio** — Async runtime (with `full` features)
- **Rust edition 2024**
