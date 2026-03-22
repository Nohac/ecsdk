# ecsdk_tracing

Tracing-to-ECS bridge for async ECS applications.

This crate captures `tracing` events, associates them with ECS entities when possible, and forwards them into the world through a channel-backed receiver resource.

## What It Covers

- `EcsTracingLayer` for `tracing_subscriber`
- `TracingReceiver` resource for draining events into the ECS world
- `LogEvent` as the captured tracing payload
- `TracingPlugin` to insert the receiver resource
- `setup(...)` to create the layer/receiver pair

## Main Pattern

`ecsdk_tracing` separates process-level tracing setup from world-level consumption:

1. Call `setup(wake_signal)`
2. Register the returned `EcsTracingLayer` with `tracing_subscriber`
3. Insert the returned `TracingReceiver` into the app with `TracingPlugin`
4. Drain `LogEvent`s into your own ECS-side log model

## Entity Association

If a tracing span records an `entity_id` field, the layer will walk the current span stack and attach that entity to emitted `LogEvent`s.

That makes it useful for:

- entity-scoped task logging
- service/container log fan-in
- building ECS-side log projections

## Patterns

- Keep tracing ingestion generic here
- Build application-specific log views in app code, not in this crate
