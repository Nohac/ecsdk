# ecsdk

Public facade crate for the `ecsdk_*` workspace.

This crate is the intended dependency for downstream applications. It re-exports the lower-level crates behind feature flags and provides a single `ecsdk::prelude::*` import for the most common Bevy ECS, async app, macro, and networking setup.

## What It Covers

- Re-exporting the workspace crates behind one dependency
- A feature-gated `prelude` for common app imports
- Re-exporting `bevy`, `serde`, and `bevy_replicon` so proc macros can target the facade instead of leaf crates

## Typical Usage

```rust
use ecsdk::prelude::*;
```

With features such as:

- `app` for `AsyncApp`, `setup`, and `run_async`
- `tasks` for entity-owned async tasks
- `term` for terminal integration
- `network` for Replicon-backed client/server support
- `tracing` for tracing-to-ECS bridging
- `macros` for derives like `StateComponent` and `ClientRequest`

## Patterns

- Prefer depending on `ecsdk` instead of individual `ecsdk_*` crates in applications
- Use feature flags to keep dependencies narrow
- Treat this crate as the stable surface; use leaf crates when working on the framework itself
