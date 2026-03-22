# ecsdk_app

Async app bootstrap and runtime loop for `ecsdk`.

This crate owns the `AsyncApp` wrapper and the Tokio-driven runtime loop that drains typed messages, world callbacks, and wake/tick signals into a Bevy `App`.

## What It Covers

- `setup<M>()` to create a ready-to-configure `AsyncApp<M>`
- `AsyncApp<M>` as a thin wrapper around `bevy::app::App`
- `run_async(...)` for the biased runtime loop
- `AppSendMsgExt` for enqueueing typed messages from `App` and `AsyncApp`

## Main Pattern

`ecsdk_app` is the runtime entrypoint for most applications:

1. Create an `AsyncApp<M>` with `setup::<M>()`
2. Add plugins, resources, and systems like a normal Bevy app
3. Call `.run().await`

Internally, the runtime loop prioritizes:

- typed state messages
- command callbacks
- immediate wake signals
- tick-driven updates

That keeps world mutations responsive while still supporting FPS-bounded rendering.

## Typical Usage

```rust
let mut app = ecsdk_app::setup::<Message>();
app.add_plugins(MyPlugin);
app.run().await;
```

## Patterns

- Use `AsyncApp` when the app owns async queues and runtime receivers
- Treat it like a normal `App` during setup; it dereferences to `App`
- Prefer `send_msg(...)` for typed domain messages during bootstrap
