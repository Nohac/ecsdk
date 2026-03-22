# ecsdk_term

Terminal integration for async ECS applications.

This crate handles terminal lifecycle setup, raw mode, alternate screen usage, terminal size tracking, and forwarding crossterm input events into the ECS world.

## What It Covers

- `TerminalGuard` for terminal setup/teardown
- `TerminalSize` resource for current dimensions
- `TermPlugin` for crossterm event forwarding
- `TerminalEvent` Bevy event
- `Rect` and scroll-region helpers for terminal layout

## Main Pattern

Use `TerminalGuard` to enter terminal mode, then `TermPlugin` to drive input into ECS:

```rust
app.insert_resource(TerminalGuard::new());
app.add_plugins(TermPlugin);
```

Resize events update `TerminalSize`, and all other crossterm events are emitted as `TerminalEvent`.

## Patterns

- Treat the terminal as another IO boundary feeding ECS events
- Use `tick()` for redraw scheduling and `wake()` for immediate input reaction
- Keep rendering systems separate from the terminal integration itself
