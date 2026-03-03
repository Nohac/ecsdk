use std::collections::HashMap;

use bevy_ecs::prelude::*;
use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use futures_util::StreamExt;
use roam_stream::{HandshakeConfig, NoDispatcher, connect};
use tokio::sync::broadcast;
use tokio::{select, signal::ctrl_c, sync::mpsc};

use crate::backend::ContainerRuntime;
use crate::backend_mirror::MirrorBackend;
use crate::bridge::{AppExit, EventSender, TokioHandle, WorldCmd};
use crate::container::*;
use crate::ipc::{ComposeDaemonClient, DaemonConnector, SOCKET_PATH};
use crate::lifecycle::{Backend, register_observers};
use crate::protocol::DaemonEvent;
use crate::render::{RenderMode, TerminalGuard, TerminalSize, build_render_schedule};

fn forward_daemon_event(
    event: Option<DaemonEvent>,
    broadcast_tx: &broadcast::Sender<DaemonEvent>,
    event_sender: &EventSender,
) {
    match event {
        Some(event) => {
            if matches!(event, DaemonEvent::Exit) {
                event_sender.send(|world| world.resource_mut::<AppExit>().0 = true);
            }
            let _ = broadcast_tx.send(event);
        }
        None => {
            // Stream ended — daemon disconnected
            event_sender.send(|world| world.resource_mut::<AppExit>().0 = true);
        }
    }
}

async fn next_term_event(events: &mut Option<EventStream>) -> Option<Result<Event, std::io::Error>> {
    match events {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

/// Run the client: connect to daemon, mirror its lifecycle locally, render.
pub async fn run_client(mode: RenderMode) {
    // RAII: terminal is restored on drop (early return, panic, etc.)
    let _terminal_guard = match mode {
        RenderMode::Tui => Some(TerminalGuard::new()),
        RenderMode::Plain => None,
    };

    // Connect to daemon
    let connector = DaemonConnector::new(SOCKET_PATH);
    let roam_client = connect(connector, HandshakeConfig::default(), NoDispatcher);
    let client = ComposeDaemonClient::new(roam_client);

    // Set up event channel for subscribe stream
    let (event_tx, mut event_rx) = roam::channel::<DaemonEvent>();

    // Start subscription — polled in select loop, no spawn needed
    let subscribe_client = client.clone();
    let mut subscribe_task = Box::pin(async move {
        let _ = subscribe_client.subscribe(event_tx).await;
    });

    // Wait for snapshot (poll subscribe to drive the connection)
    let snapshot = select! {
        result = event_rx.recv() => {
            match result {
                Ok(Some(DaemonEvent::Snapshot(snapshot))) => snapshot,
                _ => {
                    eprintln!("Failed to receive snapshot from daemon");
                    return;
                }
            }
        }
        _ = &mut subscribe_task => {
            eprintln!("Subscribe ended before snapshot");
            return;
        }
    };

    // Build MirrorBackend from snapshot
    let (broadcast_tx, _) = broadcast::channel::<DaemonEvent>(256);
    let image_to_name: HashMap<String, String> = snapshot
        .iter()
        .map(|c| (c.info.image.clone(), c.info.name.clone()))
        .collect();
    let mirror = MirrorBackend {
        events: broadcast_tx.clone(),
        image_to_name,
    };

    // Set up ECS world with full lifecycle (same as daemon, but with MirrorBackend)
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<WorldCmd>();
    let event_sender = EventSender(cmd_tx);

    let mut world = World::new();
    world.insert_resource(event_sender.clone());
    world.insert_resource(TokioHandle(tokio::runtime::Handle::current()));
    world.insert_resource(TerminalSize::query_now());
    world.init_resource::<AppExit>();
    world.init_resource::<MergedLogView>();
    world.insert_resource(Backend(ContainerRuntime::from(mirror)));

    register_observers(&mut world);

    // Spawn containers from snapshot (all Pending — lifecycle replays from start)
    for c in &snapshot {
        world.spawn((
            ContainerName(c.info.name.clone()),
            ImageRef(c.info.image.clone()),
            StartOrder(c.info.order),
            ContainerPhase::Pending,
            LogBuffer::default(),
        ));
    }
    world.spawn((
        ContainerName("[system]".to_string()),
        LogBuffer::default(),
        SystemEntity,
    ));

    let mut update = crate::lifecycle::build_update_schedule();
    let mut render = build_render_schedule(mode);

    // Kick off: first update lets enforce_ordering start the lifecycle
    update.run(&mut world);
    render.run(&mut world);

    // Terminal events — only created in Tui mode
    let mut term_events = match mode {
        RenderMode::Tui => Some(EventStream::new()),
        RenderMode::Plain => None,
    };

    loop {
        select! {
            Some(cmd) = cmd_rx.recv() => {
                cmd(&mut world);
                update.run(&mut world);
                render.run(&mut world);
                if world.resource::<AppExit>().0 { break; }
            }
            result = event_rx.recv() => {
                forward_daemon_event(result.ok().flatten(), &broadcast_tx, &event_sender);
            }
            Some(Ok(event)) = next_term_event(&mut term_events) => {
                match event {
                    Event::Resize(cols, rows) => {
                        event_sender.send(move |world| {
                            world.resource_mut::<TerminalSize>().update(cols, rows);
                        });
                    }
                    Event::Key(key)
                        if key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        let _ = client.shutdown().await;
                    }
                    _ => {}
                }
            }
            _ = ctrl_c() => {
                let _ = client.shutdown().await;
            }
            _ = &mut subscribe_task => break,
        }
    }

}
