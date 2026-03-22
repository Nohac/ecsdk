# ecsdk_core

Foundational primitives shared across the rest of the framework.

This crate defines the message and command queues that bridge async work into the ECS world, plus the wake/tick signaling model used by the main runtime loop.

## What It Covers

- `ApplyMessage` for domain messages that mutate a `World`
- `MessageQueue<M>` for typed domain events crossing the async boundary
- `CmdQueue` for arbitrary world callbacks
- `TickSignal` and `WakeSignal` for schedule control
- `AppExit` for cooperative shutdown
- `SendMsgExt` for enqueuing typed messages from `World`

## Main Pattern

The framework distinguishes between two kinds of async-to-ECS communication:

- Typed domain messages via `MessageQueue<M>`
- Ad hoc world mutations via `CmdQueue`

That separation is important:

- `ApplyMessage` is good for replayable, domain-level events
- `CmdQueue` is good for one-off callbacks that need direct world access

## Typical Usage

```rust
pub enum Message {
    SpawnThing,
}

impl ApplyMessage for Message {
    fn apply(&self, world: &mut World) {
        match self {
            Message::SpawnThing => {
                world.spawn_empty();
            }
        }
    }
}
```

## Patterns

- Use `world.send_msg(...)` for typed domain flow
- Use `CmdQueue::send(...)` for direct world callbacks
- Use `world.wake()` when a schedule run is needed immediately
- Use `world.tick()` when rendering or schedule work can wait for the next FPS boundary
