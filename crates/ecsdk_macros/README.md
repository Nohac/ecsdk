# ecsdk_macros

Proc macros for the public `ecsdk` facade.

These derives generate framework glue so application crates can stay focused on domain code instead of repetitive registration and marker boilerplate.

## What It Covers

- `#[derive(StateComponent)]` for enum-backed marker state machines
- `#[derive(ClientRequest)]` for request/response type declarations

## `StateComponent`

`StateComponent` is intended for fieldless enums that represent lifecycle or phase state.

It generates:

- one marker component per enum variant
- helper methods to insert those markers
- `replicate_markers(...)` for Replicon setup
- marker `on_insert` hooks that synchronize the enum component and request a wake when `WakeSignal` is present

This is useful when the server wants marker-driven state transitions but clients still need the same marker model replicated.

## `ClientRequest`

`ClientRequest` is intended for event types with an associated response:

```rust
#[derive(Event, ClientRequest, Serialize, Deserialize)]
#[request(response = "StatusResponse")]
pub struct StatusRequest;
```

It generates:

- the `ecsdk::network::ClientRequest` impl
- inherent `register(...)`
- inherent `reply(...)`

## Important Note

These macros assume downstream crates depend on the public `ecsdk` facade crate. Generated code references `ecsdk::...` rather than leaf crates directly.
