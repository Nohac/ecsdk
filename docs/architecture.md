# Architecture: Event-Driven ECS with Async Runtime

This document describes an architecture for building event-driven applications using Bevy ECS as a data-driven application framework, integrated with a Tokio async runtime. The architecture is generic — the container orchestration system in this project is one concrete application of these patterns.

## 1. Core Idea: ECS Without a Game Loop

Traditional Bevy apps call `App::run()`, which starts a fixed-rate game loop. This architecture discards that in favor of **manual, event-driven updates**:

- The `World` is only updated when something meaningful happens (a message arrives, an async task reports progress, a user presses a key)
- There is no fixed frame rate for logic — only an optional tick rate for rendering
- `app.update()` is called explicitly by the main loop, not by a timer

This makes Bevy ECS suitable for long-running services, CLI tools, orchestrators, and other non-game applications where work is driven by external events rather than a continuous simulation.

```
┌──────────────────────────────────────────────────────────┐
│                    Traditional Bevy                       │
│  App::run() → fixed tick → systems run every frame       │
├──────────────────────────────────────────────────────────┤
│                    This Architecture                      │
│  Event arrives → apply to world → app.update() → sleep   │
│  Nothing happens → nothing runs                          │
└──────────────────────────────────────────────────────────┘
```


## 2. The Async-ECS Bridge

Bevy ECS is synchronous. Tokio is async. They meet through two mpsc channels:

```
                    ┌─────────────┐
  Async Tasks ──────┤  CmdQueue   ├──► WorldCallback closures ──► World
                    └─────────────┘
                    ┌─────────────┐
  Async Tasks ──────┤ MessageQueue├──► Serializable events ─────► World
                    └─────────────┘
```

### Channel 1: WorldCallbacks (internal mutations)

A `WorldCallback` is a boxed closure that mutates the world directly:

```rust
pub type WorldCallback = Box<dyn FnOnce(&mut World) + Send>;
```

`CmdQueue` wraps a `mpsc::UnboundedSender<WorldCallback>` and provides a fluent API:

```rust
#[derive(Resource, Clone)]
pub struct CmdQueue {
    tx: mpsc::UnboundedSender<WorldCallback>,
    handle: Option<Handle>,  // Tokio runtime handle for task spawning
    wake: WakeSignal,
}

impl CmdQueue {
    pub fn send(&self, f: impl FnOnce(&mut World) + Send + 'static) -> &Self {
        let _ = self.tx.send(Box::new(f));
        self
    }

    pub fn wake(&self) {
        self.wake.0.notify_one();
    }
}
```

Chaining allows grouping multiple mutations with a single wake:

```rust
cmd.send(|world| { /* update progress */ })
   .send(|world| { /* insert component */ })
   .wake();
```

### Channel 2: Messages (domain events)

A `Message` is a serializable enum that represents a domain event. Each variant implements `apply(&self, &mut World)` to mutate the ECS:

```rust
#[derive(Serialize, Deserialize)]
pub enum Message {
    SpawnEntity { name: String, config: Config },
    MarkDone { entity_name: String },
    RequestShutdown,
}

impl Message {
    pub fn apply(&self, world: &mut World) {
        match self {
            Self::SpawnEntity { name, config } => {
                let mut entity = world.spawn((
                    Name(name.clone()),
                    config.clone(),
                ));
                Phase::Pending.insert_marker_world(&mut entity);
            }
            Self::MarkDone { entity_name } => { /* find and mark entity */ }
            Self::RequestShutdown => { /* set flag */ }
        }
    }
}
```

### Why two channels?

| | WorldCallbacks | Messages |
|---|---|---|
| **Format** | Closures (`FnOnce(&mut World)`) | Serializable enums |
| **Use case** | Internal plumbing (progress, logs) | Domain events |
| **IPC-safe** | No (contains pointers) | Yes (can cross process boundaries) |
| **Testable** | No (opaque closures) | Yes (can record and replay) |

The separation keeps domain events clean and transportable while still allowing raw world access for internal mutations.


## 3. The Select Loop

The heart of the architecture is a `tokio::select!` loop that dispatches events to the ECS world:

```rust
pub async fn run_async(mut app: App, mut rx: Receivers) {
    let mut tick_interval = interval(Duration::from_millis(1000 / 5));
    let mut needs_tick = false;

    app.finish();
    app.cleanup();
    app.update();

    loop {
        tokio::select! {
            biased;

            // Priority 1: State events (domain messages)
            Some(event) = rx.state_rx.recv() => {
                event.apply(app.world_mut());
                while let Ok(event) = rx.state_rx.try_recv() {
                    event.apply(app.world_mut());
                }
                drain_cmds(app.world_mut(), &mut rx.cmd_rx);
                app.update();
            }

            // Priority 2: World callbacks from async tasks
            Some(cb) = rx.cmd_rx.recv() => {
                cb(app.world_mut());
                drain_cmds(app.world_mut(), &mut rx.cmd_rx);
            }

            // Priority 3: Wake signal → immediate update
            _ = rx.wake.notified() => {
                drain_cmds(app.world_mut(), &mut rx.cmd_rx);
                app.update();
            }

            // Priority 4: Tick request → deferred to next FPS boundary
            _ = rx.tick.notified() => {
                needs_tick = true;
            }

            // Priority 5: FPS timer fires (only if tick was requested)
            _ = tick_interval.tick(), if needs_tick => {
                drain_cmds(app.world_mut(), &mut rx.cmd_rx);
                app.update();
                needs_tick = false;
            }
        }

        if app.world().resource::<AppExit>().0 {
            break;
        }
    }
}
```

### Key design choices

**`biased` select**: Ensures state events are always processed before ticks. Without this, Tokio would randomly pick a ready branch, and a burst of tick signals could starve state events.

**Wake vs Tick**: Two scheduling signals serve different purposes:
- **Wake** (`Notify`) triggers an immediate `app.update()`. Used when something important happened that needs to be visible right away (task completion, user input).
- **Tick** (`Notify`) requests an update at the next FPS boundary. Used for cosmetic updates (progress bars, log lines) that can batch together.

**Drain pattern**: After receiving any event, `drain_cmds` pulls all pending callbacks before calling `app.update()`. This coalesces multiple mutations into a single update cycle.

### Setup

The `setup()` function creates the channels and inserts them as ECS resources:

```rust
pub fn setup() -> (App, Receivers) {
    let (state_tx, state_rx) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let tick = Arc::new(Notify::new());
    let wake = Arc::new(Notify::new());

    let mut app = App::new();
    app.insert_resource(CmdQueue::new(cmd_tx, Handle::current(), WakeSignal(wake.clone())));
    app.insert_resource(MessageQueue::new(state_tx));
    app.insert_resource(TickSignal(tick.clone()));
    app.insert_resource(WakeSignal(wake.clone()));
    app.init_resource::<AppExit>();

    (app, Receivers { state_rx, cmd_rx, tick, wake })
}
```


## 4. Entity-Scoped Async Tasks

Async tasks are tied to entities. When an entity is despawned, its task is automatically cancelled.

### The SpawnTask trait

An extension trait on `EntityCommands` provides the spawning API:

```rust
pub trait SpawnTask {
    fn spawn_task<F, Fut>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(TaskQueue) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static;
}
```

### TaskQueue — the task's handle to the world

The closure receives a `TaskQueue` that bundles the owning entity, a `CmdQueue` for world mutations, and a `MessageQueue` for domain events:

```rust
pub struct TaskQueue {
    entity: Entity,
    queue: CmdQueue,
    state_queue: MessageQueue,
}

impl TaskQueue {
    pub fn entity(&self) -> Entity { self.entity }
    pub fn send(&self, f: impl FnOnce(&mut World) + Send + 'static) -> &Self { ... }
    pub fn send_state(&self, event: Message) { ... }
    pub fn wake(&self) { ... }
}
```

### Lifecycle

When `spawn_task` is called:

1. A `CancellationToken` is created
2. The async closure is invoked with a `TaskQueue`
3. The future is spawned onto Tokio with a `select!` between the task future and the cancellation token
4. An `AsyncTask` component (holding the token and join handle) is inserted on the entity
5. On completion: a `TaskComplete` event is triggered via world callback
6. On cancellation (entity despawned → `AsyncTask` dropped → token cancelled): a `TaskAborted` event is triggered

```rust
#[derive(Component)]
pub struct AsyncTask {
    token: CancellationToken,
    _handle: JoinHandle<()>,
}

impl Drop for AsyncTask {
    fn drop(&mut self) {
        self.token.cancel();  // Automatic cleanup
    }
}
```

### Usage pattern

```rust
commands.entity(entity).spawn_task(move |cmd| async move {
    let entity = cmd.entity();

    // Do async work, reporting progress along the way
    let progress_cmd = cmd.clone();
    some_async_operation(move |bytes| {
        let downloaded = bytes;
        progress_cmd.send(move |world: &mut World| {
            if let Some(mut progress) = world.get_mut::<Progress>(entity) {
                progress.downloaded = downloaded;
            }
            world.tick();  // Request render update
        });
    }).await;

    // Signal completion via domain event
    cmd.send_state(Message::MarkDone { entity_name });
});
```

The task sends incremental progress via world callbacks (with `tick()` for batched rendering updates) and signals completion via a domain message (which triggers an immediate state machine transition).


## 5. State Machines with seldom_state

State machines are built using the `seldom_state` crate, which integrates with Bevy ECS. The current pattern is:

- Define a canonical enum component for the generic lifecycle (`Phase`)
- Derive `StateComponent` on that enum
- Let the derive generate zero-size marker components in a snake_case module (`phase::Pending`, `phase::Running`, etc.)
- Let generated `on_insert` hooks mirror the inserted marker back into the enum component

### Defining states

The enum is the source of truth for generic querying, serialization, replay, and rendering:

```rust
#[derive(Component, StateComponent, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pending,
    Processing,
    Running,
    Stopping,
    Stopped,
}
```

The derive generates marker components roughly like:

```rust
pub mod phase {
    #[derive(Component, Clone)]
    #[component(on_insert = on_insert_pending)]
    pub struct Pending;

    #[derive(Component, Clone)]
    #[component(on_insert = on_insert_processing)]
    pub struct Processing;

    // ...
}
```

When `phase::Processing` is inserted by the state machine, the generated hook writes `Phase::Processing` onto the entity. That keeps the enum queryable without manual `.on_enter(... insert(Phase::...))` boilerplate.

The derive also adds helper methods on the enum itself for initial state insertion:

```rust
let mut entity = world.spawn((Name("worker".into()), build_entity_sm()));
Phase::Pending.insert_marker_world(&mut entity);
```

### Writing trigger predicates

Triggers are system-like functions that receive `In(entity)` and queries, returning `bool`:

```rust
// Transition when all predecessors have finished
fn predecessors_ready(
    In(entity): In<Entity>,
    this: Query<&StartOrder>,
    all: Query<(&StartOrder, Has<Running>, Has<Stopped>)>,
) -> bool {
    let Ok(order) = this.get(entity) else { return false };
    all.iter().all(|(other_order, is_running, is_stopped)| {
        if other_order.0 < order.0 {
            is_running || is_stopped
        } else {
            true
        }
    })
}

// Transition when the entity has a Done marker
fn has_done(In(entity): In<Entity>, dones: Query<&Done>) -> bool {
    dones.get(entity).is_ok()
}

// Transition based on a global resource flag
fn shutdown_requested(In(_entity): In<Entity>, flag: Res<ShutdownRequested>) -> bool {
    flag.0
}
```

### Building the state machine

The builder pattern only needs to describe legal transitions:

```rust
use crate::phase::{Pending, Processing, Running, Stopped, Stopping};

pub fn build_entity_sm() -> StateMachine {
    StateMachine::default()
        .trans::<Pending, _>(predecessors_ready, Processing)
        .trans::<Processing, _>(has_done, Running)
        // Shutdown can interrupt any active state
        .trans::<OneOfState<(Pending, Processing, Running)>, _>(
            shutdown_requested,
            Stopping,
        )
        .trans::<Stopping, _>(has_done, Stopped)
        .set_trans_logging(true)
}
```

### Two-level state machine pattern

A common pattern is combining entity-level state machines with a global **orchestrator** state machine that watches aggregate state:

```rust
// Orchestrator triggers check ALL entities
fn all_entities_running(
    In(_entity): In<Entity>,
    entities: Query<Has<Running>, With<Managed>>,
) -> bool {
    !entities.is_empty() && entities.iter().all(|r| r)
}

fn all_entities_stopped(
    In(_entity): In<Entity>,
    entities: Query<Has<Stopped>, With<Managed>>,
) -> bool {
    !entities.is_empty() && entities.iter().all(|s| s)
}

pub fn build_orchestrator_sm() -> StateMachine {
    StateMachine::default()
        .trans::<Deploying, _>(all_entities_running, AllRunning)
        .trans::<OneOfState<(Deploying, AllRunning)>, _>(shutdown_requested, ShuttingDown)
        .trans::<ShuttingDown, _>(all_entities_stopped, AllStopped)
        .set_trans_logging(true)
}
```

The orchestrator entity watches the aggregate state of all managed entities and drives global transitions (e.g., "all ready" → set a flag, "all stopped" → exit the app).

In practice this pattern works well for both entity-local and global lifecycles:

```rust
#[derive(Component, StateComponent, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrchestratorPhase {
    Deploying,
    AllRunning,
    ShuttingDown,
    AllStopped,
}
```

### Plugin registration

The `StateMachinePlugin` must be added for transitions to evaluate:

```rust
app.add_plugins(StateMachinePlugin::default().schedule(PreUpdate));
```


## 6. Observer-Driven Side Effects

State transitions are declarative (triggers + target states). **Side effects** — the actual work that happens on state entry — are registered separately as Bevy observers. This decouples "what transitions exist" from "what happens on transition."

### Registering observers

```rust
pub struct LifecyclePlugin;

impl Plugin for LifecyclePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(StateMachinePlugin::default().schedule(PreUpdate));
        app.add_observer(on_processing);
        app.add_observer(on_running);
        app.add_observer(on_stopping);
        app.add_observer(on_stopped);
    }
}
```

### Observer handlers

An observer fires when its target component is inserted. The handler typically spawns an async task:

```rust
fn on_processing(
    trigger: On<Insert, Processing>,
    mut commands: Commands,
    backends: Query<&Backend>,
    names: Query<&Name>,
) {
    let entity = trigger.event_target();
    let backend = backends.get(entity).unwrap().0.clone();
    let name = names.get(entity).map(|n| n.0.clone()).unwrap_or_default();

    commands.entity(entity).spawn_task(move |cmd| async move {
        let entity = cmd.entity();
        let progress_cmd = cmd.clone();

        backend.do_work(move |progress| {
            progress_cmd.send(move |world: &mut World| {
                if let Some(mut p) = world.get_mut::<Progress>(entity) {
                    p.value = progress;
                }
                world.tick();
            });
        }).await;

        cmd.send_state(Message::MarkDone { entity_name: name });
    });
}
```

### The cycle

```
State machine transition
    → Component inserted (e.g., Processing)
        → Observer fires
            → Async task spawned
                → Task does work, sends progress via callbacks
                    → Task sends MarkDone message
                        → Message applies Done marker to entity
                            → State machine sees Done, transitions to next state
                                → Next observer fires...
```

### Testability benefit

Because observers are registered in the plugin, you can create a **test plugin** that registers the state machine infrastructure without any observers. This lets you test state machine logic in isolation by manually inserting `Done` markers:

```rust
pub struct TestPlugin;

impl Plugin for TestPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(StateMachinePlugin::default().schedule(PreUpdate));
        // No observers — pure state machine testing
    }
}
```


## 7. The Message-Apply Pattern

Domain events are modeled as a serializable enum where each variant knows how to apply itself to the world:

```rust
#[derive(Serialize, Deserialize)]
pub enum Message {
    SpawnEntity { name: String, image: String, order: u32 },
    MarkDone { entity_name: String },
    RequestShutdown,
}

impl Message {
    pub fn apply(&self, world: &mut World) {
        match self { ... }
    }
}
```

### Properties

1. **Serializable**: Messages can be sent over IPC, stored to disk, or transmitted over the network
2. **Replayable**: A sequence of messages deterministically produces a world state
3. **Testable**: Feed a message sequence to a fresh world and assert the result

### Event replay for testing

```rust
pub fn replay(events: &[Message]) -> App {
    let mut app = App::new();
    app.add_plugins(TestPlugin);
    app.update();

    for event in events {
        event.apply(app.world_mut());
        app.update();
    }

    app
}

#[test]
fn full_lifecycle_via_replay() {
    let events = vec![
        Message::SpawnEntity { name: "worker".into(), image: "img".into(), order: 0 },
        Message::MarkDone { entity_name: "worker".into() },  // Processing → Running
        Message::MarkDone { entity_name: "worker".into() },  // Starting → Running
        Message::RequestShutdown,
        Message::MarkDone { entity_name: "worker".into() },  // Stopping → Stopped
    ];

    let mut app = replay(&events);
    // Assert final state...
}
```

### JSON round-trip

Because messages are `Serialize + Deserialize`, you can record a session and replay it:

```rust
// Record
let json: Vec<String> = events.iter()
    .map(|e| serde_json::to_string(e).unwrap())
    .collect();

// Replay
let deserialized: Vec<Message> = json.iter()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect();

let app = replay(&deserialized);
```


## 8. Client-Server with Replicon

For applications that need a daemon/client split, `bevy_replicon` provides server-authoritative ECS replication. The architecture uses a custom transport layer over Unix sockets (or any `AsyncRead + AsyncWrite` stream).

### Shared registration

A shared plugin ensures both sides register components and events in identical order (required by Replicon):

```rust
pub struct SharedReplicationPlugin;

impl Plugin for SharedReplicationPlugin {
    fn build(&self, app: &mut App) {
        // Components replicated automatically
        app.replicate::<Name>();
        app.replicate::<Phase>();
        app.replicate::<Progress>();

        // Events sent explicitly
        app.add_mapped_server_event::<LogEvent>(Channel::Ordered);
        app.add_server_event::<ServerExitNotice>(Channel::Ordered);
        app.add_client_event::<ShutdownRequest>(Channel::Ordered);
    }
}
```

### Packet format

Replicon packets are wrapped in a simple format for transport:

```
┌─────────────────────────────────────────┐
│ Length-delimited frame                  │
│ ┌─────────────┬───────────────────────┐ │
│ │ channel_id  │ data                  │ │
│ │ (1 byte)    │ (replicon payload)    │ │
│ └─────────────┴───────────────────────┘ │
└─────────────────────────────────────────┘
```

### Bidirectional bridge

The core transport is a generic bridge function that works with any async stream:

```rust
async fn run_bridge(
    stream: impl AsyncRead + AsyncWrite + Send + Unpin,
    to_remote_rx: &mut mpsc::UnboundedReceiver<RepliconPacket>,
    from_remote_tx: &mpsc::UnboundedSender<RepliconPacket>,
    wake: impl Fn(),
) {
    let (mut sink, mut source) = Framed::new(stream, LengthDelimitedCodec::new()).split();

    let send_to_remote = async {
        while let Some(packet) = to_remote_rx.recv().await {
            if sink.send(packet.encode()).await.is_err() { break; }
        }
    };

    let recv_from_remote = async {
        while let Some(Ok(frame)) = source.next().await {
            if let Some(packet) = RepliconPacket::decode(frame.into()) {
                let _ = from_remote_tx.send(packet);
                wake();
            }
        }
    };

    tokio::select! {
        _ = send_to_remote => {}
        _ = recv_from_remote => {}
    }
}
```

### Server transport

The server maintains a `HashMap<Entity, channels>` for multiple simultaneous clients:

```rust
#[derive(Resource, Default)]
struct ServerBridge {
    clients: HashMap<Entity, ServerClientChannels>,
}
```

A listener task accepts connections and spawns per-client bridge tasks:

```rust
fn spawn_server_listener(mut commands: Commands) {
    commands.spawn_empty().spawn_task(move |cmd| async move {
        let listener = create_listener().expect("Failed to bind");
        loop {
            let stream = listener.accept().await.unwrap();
            cmd.send(move |world: &mut World| {
                AcceptClientCmd { stream }.apply(world);
            }).wake();
        }
    });
}
```

`AcceptClientCmd` spawns a client entity with `ConnectedClient` (Replicon marker) and a bridge task:

```rust
impl Command for AcceptClientCmd {
    fn apply(self, world: &mut World) {
        let (to_client_tx, to_client_rx) = mpsc::unbounded_channel();
        let (from_client_tx, from_client_rx) = mpsc::unbounded_channel();

        let mut com = world.commands();
        let mut client = com.spawn(ConnectedClient { max_size: 1200 });
        let client_id = client.id();

        client.spawn_task(move |cmd| async move {
            let mut to_client_rx = to_client_rx;
            let wake = cmd.clone();
            run_bridge(stream, &mut to_client_rx, &from_client_tx, move || {
                wake.wake();
            }).await;

            // Bridge closed — unregister client
            let entity = cmd.entity();
            cmd.send(move |world: &mut World| {
                UnregisterClientCmd { entity }.apply(world);
            }).wake();
        });

        world.flush();
        world.resource_mut::<ServerBridge>().clients.insert(client_id, ...);
    }
}
```

### Client transport

The client spawns a single connection task:

```rust
fn spawn_client_connection(mut commands: Commands) {
    commands.spawn_empty().spawn_task(move |cmd| async move {
        let stream = connect().await.unwrap();
        let (to_server_tx, mut to_server_rx) = mpsc::unbounded_channel();
        let (from_server_tx, from_server_rx) = mpsc::unbounded_channel();

        // Insert bridge resource so ECS systems can route packets
        cmd.send(move |world: &mut World| {
            InsertClientBridgeCmd { from_server_rx, to_server_tx }.apply(world);
        }).wake();

        let wake = cmd.clone();
        run_bridge(stream, &mut to_server_rx, &from_server_tx, move || {
            wake.wake();
        }).await;

        // Connection lost — exit
        cmd.send(|world: &mut World| {
            world.resource_mut::<AppExit>().0 = true;
        }).wake();
    });
}
```

### System scheduling

Replicon expects packet I/O in specific system sets:

```rust
// Server
app.add_systems(PreUpdate,
    (server_manage_state, server_receive_packets)
        .chain()
        .in_set(ServerSystems::ReceivePackets),
);
app.add_systems(PostUpdate,
    server_send_packets.in_set(ServerSystems::SendPackets),
);

// Client
app.add_systems(PreUpdate,
    (client_manage_state, client_receive_packets)
        .chain()
        .in_set(ClientSystems::ReceivePackets),
);
app.add_systems(PostUpdate,
    client_send_packets.in_set(ClientSystems::SendPackets),
);
```

### Connection state management

Both server and client manage connection state with Bevy's `States`:

```rust
fn client_manage_state(
    bridge: Option<Res<ClientBridge>>,
    state: Res<State<ClientState>>,
    mut next_state: ResMut<NextState<ClientState>>,
) {
    match (bridge.is_some(), state.get()) {
        (true, &ClientState::Disconnected) => next_state.set(ClientState::Connected),
        (false, &ClientState::Connected) => next_state.set(ClientState::Disconnected),
        _ => {}
    }
}
```

### What to replicate

| Type | Mechanism | Direction |
|---|---|---|
| Entity components (name, phase, progress) | `app.replicate::<T>()` + `Replicated` marker | Server → Client |
| Log events | `app.add_mapped_server_event::<T>()` | Server → Client |
| Exit notices | `app.add_server_event::<T>()` | Server → Client |
| Shutdown requests | `app.add_client_event::<T>()` | Client → Server |

`mapped` events use `MapEntities` so entity references in the event are remapped to the client's entity IDs.


## 9. Resource and Component Design Principles

### States as enum + generated marker components

The enum component is the generic read model, while the generated marker components are the ECS-native indexing/query layer that `seldom_state` transitions over.

Query the enum when you want the generic lifecycle:

```rust
fn render(query: Query<&Phase>) {
    for phase in &query {
        match phase {
            Phase::Pending => {}
            Phase::Running => {}
            _ => {}
        }
    }
}
```

Query the generated marker when you want a state-specific filter:

```rust
fn count_running(query: Query<Entity, With<phase::Running>>) -> usize {
    query.iter().count()
}
```

### Transient components

Components that only exist during a specific phase are inserted on state entry and implicitly replaced or removed on state exit:

```rust
fn on_processing(trigger: On<Insert, Processing>, mut commands: Commands) {
    commands.entity(trigger.event_target()).insert(DownloadProgress {
        downloaded: 0,
        total: 0,
    });
}
```

### Global coordination via resources

Boolean flag resources coordinate between entity-level and global-level logic:

```rust
#[derive(Resource, Default)]
pub struct ShutdownRequested(pub bool);

#[derive(Resource, Default)]
pub struct AppExit(pub bool);
```

### ECS as single source of truth

Async tasks **never hold authoritative state**. They push updates into the ECS world via callbacks. If you need to know the current state of an entity, query the world — don't ask the task.

```
❌  Task holds state, world reads from task
✅  Task computes, pushes results into world, world is the source of truth
```


## 10. Rendering as a System

Rendering is just another system that queries ECS state. It has no special relationship with the rest of the architecture.

### Render system pattern

```rust
pub fn render(
    query: Query<(Entity, &Name, &Phase, Option<&Progress>)>,
    logs: Res<MergedLogView>,
    term_size: Res<TerminalSize>,
    mut state: Local<RenderState>,  // Tracks what was last drawn
) {
    // Delta rendering: compare current state to last-drawn state
    // Only redraw what changed
}
```

### Multiple render backends

Different render modes are registered as different system sets:

```rust
match mode {
    RenderMode::Tui => {
        app.insert_resource(TerminalGuard::new());
        app.insert_resource(TerminalSize::query_now());
        app.add_systems(PostUpdate, render_tui);
    }
    RenderMode::Plain => {
        app.add_systems(PostUpdate, render_plain);
    }
    RenderMode::None => {}
}
```

### Terminal events as ECS events

Terminal input is fed back into the ECS as events via the same async task pattern:

```rust
fn setup_crossterm(mut commands: Commands) {
    commands.spawn(CrosstermEntity).spawn_task(|cmd| async move {
        let mut events = EventStream::new();
        while let Some(Ok(event)) = events.next().await {
            if let Event::Resize(cols, rows) = event {
                cmd.send(move |world: &mut World| {
                    let mut size = world.resource_mut::<TerminalSize>();
                    size.cols = cols;
                    size.rows = rows;
                    world.tick();
                });
            }
            let event_clone = event.clone();
            cmd.send(move |world: &mut World| {
                world.trigger(TerminalEvent(event_clone));
            }).wake();
        }
    });
}
```

Resize events use `tick()` (batched, cosmetic). Key events use `wake()` (immediate, needs response).

### Delta rendering with Local state

The render system uses `Local<RenderState>` to track what was last drawn and skip unchanged elements:

```rust
#[derive(Default)]
struct RenderState {
    last_phase: HashMap<Entity, Phase>,
    last_progress: HashMap<Entity, u64>,
    log_cursor: usize,
}
```


## 11. Testability via Event Replay

The Message-Apply pattern enables deterministic testing without async infrastructure.

### Test plugin

A stripped-down plugin that has the state machine infrastructure but no observers (no async tasks, no side effects):

```rust
pub struct TestPlugin;

impl Plugin for TestPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(StateMachinePlugin::default().schedule(PreUpdate));
        // Resources needed by triggers
        app.init_resource::<ShutdownRequested>();
    }
}
```

Because enum synchronization is handled by the generated component hooks, the test plugin does not need any extra registration for phase mirroring.

### Replay function

```rust
pub fn replay(events: &[Message]) -> App {
    let mut app = App::new();
    app.add_plugins(TestPlugin);
    app.update();

    for event in events {
        event.apply(app.world_mut());
        app.update();
    }

    app
}
```

### Unit tests drive state machines directly

Without observers, you manually insert `Done` markers to simulate task completion:

```rust
#[test]
fn full_lifecycle() {
    let mut app = test_app();
    let entity = spawn_entity(&mut app, 0, phase::Pending);

    app.update(); // Pending → Processing
    assert!(app.world().get::<phase::Processing>(entity).is_some());
    assert_eq!(app.world().get::<Phase>(entity), Some(&Phase::Processing));

    app.world_mut().entity_mut(entity).insert(Done::Success);
    app.update(); // Processing → Running
    assert!(app.world().get::<phase::Running>(entity).is_some());

    app.world_mut().resource_mut::<ShutdownRequested>().0 = true;
    app.update(); // Running → Stopping

    app.world_mut().entity_mut(entity).insert(Done::Success);
    app.update(); // Stopping → Stopped
    assert!(app.world().get::<phase::Stopped>(entity).is_some());
    assert_eq!(app.world().get::<Phase>(entity), Some(&Phase::Stopped));
}
```

This tests the entire state machine without any async runtime, network, or real backends.


## Summary: Event Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Async Task                                  │
│  1. Does work (network, I/O, computation)                          │
│  2. Sends progress via cmd.send(closure).tick()                    │
│  3. Sends completion via cmd.send_state(Message::MarkDone)         │
└─────────────────────┬─────────────────────┬─────────────────────────┘
                      │                     │
              WorldCallback            Message
                      │                     │
                      ▼                     ▼
┌─────────────────────────────────────────────────────────────────────┐
│                      Select Loop                                    │
│  Receives callbacks and messages                                   │
│  Applies them to the World                                         │
│  Calls app.update()                                                │
└─────────────────────────────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     app.update()                                    │
│  1. StateMachinePlugin evaluates triggers (PreUpdate)              │
│  2. State transitions fire → components inserted/removed            │
│  3. Observers fire on component insertion                          │
│  4. Observers spawn new async tasks                                │
│  5. Render systems query world state (PostUpdate)                  │
└─────────────────────────────────────────────────────────────────────┘
                      │
                      ▼
              New async tasks spawned → cycle repeats
```
