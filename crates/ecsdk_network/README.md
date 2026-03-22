# ecsdk_network

Replicon-backed networking and isomorphic app composition for `ecsdk`.

This crate contains the Bevy Replicon integration layer, client/server transport plugins, request/response helpers, and the `IsomorphicApp` builder used to define shared client/server features once and build role-specific apps from them.

## What It Covers

- `IsomorphicApp<M>` for staged client/server app assembly
- `IsomorphicPlugin` for co-located shared, client, and server behavior
- `RequestPlugin` for common client-request/server-response flows
- `ClientRequest` for typed request/response registration and reply helpers
- `ClientRepliconPlugin` / `ServerRepliconPlugin` for app setup
- `ConnectClientCmd` / `AcceptClientCmd<S>` for wiring streams into Replicon
- `InitialConnection` and `Connected` markers on the replicated connection-state entity

## Main Pattern

Define shared features once, then build a server or client app from the same staged definition:

```rust
let mut iso = IsomorphicApp::<Message>::new();
iso.add_plugin(MyFeature);

let client = iso.build_client();
let server = iso.build_server();
```

Each `IsomorphicPlugin` can contribute:

- `build_shared(...)`
- `build_server(...)`
- `build_client(...)`

## Request / Response Pattern

For Replicon RPC-like flows, use `ClientRequest` plus `RequestPlugin`:

- shared registration of request and response events
- server-side request handling
- optional client-side auto-send on a trigger component

This works well for one-shot commands such as status or control requests, while persistent state should still be modeled as replicated ECS state.

## Transport Structure

The lower-level bridge lives in `ecsdk_transport`. This crate layers on:

- Replicon message routing
- Bevy app/plugin setup
- connection state markers
- role-specific client/server wiring

## Patterns

- Treat replicated components as canonical shared state
- Use request/response events for command-like interactions
- Keep feature code co-located with `IsomorphicPlugin`
