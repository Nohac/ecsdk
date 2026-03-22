# ecsdk_tasks

Entity-owned async task spawning for Bevy ECS worlds.

This crate makes async work feel like an extension of ECS entities. Tasks are attached to entities, cancelled when those entities are dropped, and can report results back through typed messages or direct world callbacks.

## What It Covers

- `SpawnTask<M>` for async tasks with typed message support
- `SpawnCmdTask` for callback-only tasks
- `TaskQueue<M>` as the async handle passed into spawned tasks
- `CmdOnly` for callback-only task handles
- `AsyncTask` for lifecycle tracking and cancellation
- `TaskComplete` / `TaskAborted` lifecycle events

## Main Pattern

Spawn async work from `EntityCommands` and keep ownership on the ECS side:

```rust
commands.entity(entity).spawn_task(|task| async move {
    task.send_state(Message::Finished);
});
```

When the owning entity is despawned, the task is cancelled automatically.

## When To Use Which API

- `SpawnTask<M>` when the task emits typed domain messages
- `SpawnCmdTask` when the task only needs `World` callbacks
- `TaskQueue<M>::send_state(...)` for domain-level outcomes
- `TaskQueue<M>::send(...)` for direct one-off world access

## Patterns

- Keep the owning entity as the identity boundary for async work
- Prefer typed messages for durable domain flow
- Use command callbacks for glue code and one-off ECS mutations
