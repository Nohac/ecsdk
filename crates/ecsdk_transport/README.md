# ecsdk_transport

Pure async transport glue for Replicon-style packet streams.

This crate has no Bevy dependency. It only defines the packet framing and bidirectional stream bridge used by the higher-level networking crate.

## What It Covers

- `RepliconPacket` as a framed channel-id plus payload packet
- `run_bridge(...)` for wiring an `AsyncRead + AsyncWrite` stream to mpsc channels

## Main Pattern

`run_bridge(...)` sits between:

- a framed byte stream such as a TCP or Unix socket
- an outbound packet channel
- an inbound packet channel

The higher-level networking crate decides what those packets mean. This crate only handles framing and transport flow.

## Patterns

- Keep transport concerns separate from Bevy app composition
- Use this crate when you want the bridge without pulling in ECS dependencies
