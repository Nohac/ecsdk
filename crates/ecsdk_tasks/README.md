# ecsdk_tasks

Entity-owned async task spawning for Bevy ECS worlds.

This crate makes async work feel like an extension of ECS entities. Tasks are attached to entities, cancelled when those entities are dropped, and can report results back through typed messages or direct world callbacks.

## What It Covers

- `SpawnTask` for entity-owned async tasks
- `TaskQueue` as the async handle passed into spawned tasks
- `AsyncTask` for lifecycle tracking and cancellation
- `TaskComplete` / `TaskAborted` lifecycle events

## Main Pattern

Spawn async work from `EntityCommands` and keep ownership on the ECS side:

```rust
commands.entity(entity).spawn_task(|task| async move {
    task.send_msg(Message::Finished);
});
```

When the owning entity is despawned, the task is cancelled automatically.

## TaskQueue API

- `TaskQueue::send_msg(...)` for domain-level outcomes
- `TaskQueue::queue_cmd(...)` for direct one-off world access
- `TaskQueue::wake()` when the task needs an immediate schedule run

## Patterns

- Keep the owning entity as the identity boundary for async work
- Prefer typed messages for durable domain flow
- Use command callbacks for glue code and one-off ECS mutations
