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
use crate::render::{
    RenderMode, TerminalSize, build_render_schedule, install_panic_hook, terminal_init,
    terminal_teardown,
};

/// Run the client: connect to daemon, mirror its lifecycle locally, render.
pub async fn run_client(mode: RenderMode) {
    if mode == RenderMode::Tui {
        install_panic_hook();
        terminal_init();
    }

    // Connect to daemon
    let connector = DaemonConnector::new(SOCKET_PATH);
    let roam_client = connect(connector, HandshakeConfig::default(), NoDispatcher);
    let client = ComposeDaemonClient::new(roam_client);

    // Set up event channel for subscribe stream
    let (event_tx, mut event_rx) = roam::channel::<DaemonEvent>();

    let subscribe_client = client.clone();
    tokio::spawn(async move {
        let _ = subscribe_client.subscribe(event_tx).await;
    });

    // Wait for snapshot
    let snapshot = match event_rx.recv().await {
        Ok(Some(DaemonEvent::Snapshot(snapshot))) => snapshot,
        _ => {
            if mode == RenderMode::Tui {
                terminal_teardown();
            }
            eprintln!("Failed to receive snapshot from daemon");
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

    // Forwarding task: roam → broadcast (+ handle Exit/disconnect)
    let fwd_sender = event_sender.clone();
    tokio::spawn(async move {
        while let Ok(Some(event)) = event_rx.recv().await {
            if matches!(event, DaemonEvent::Exit) {
                fwd_sender.send(|world| world.resource_mut::<AppExit>().0 = true);
            }
            let _ = broadcast_tx.send(event);
        }
        // Stream ended — daemon disconnected
        fwd_sender.send(|world| world.resource_mut::<AppExit>().0 = true);
    });

    // Terminal events
    let mut term_events = EventStream::new();

    loop {
        select! {
            // Single render path: all world mutations arrive here
            Some(cmd) = cmd_rx.recv() => {
                cmd(&mut world);
                update.run(&mut world);
                render.run(&mut world);
                if world.resource::<AppExit>().0 { break; }
            }
            // Resize flows through cmd channel → rendered in the arm above
            // Ctrl+C triggers daemon shutdown directly (no render needed)
            Some(Ok(event)) = term_events.next(), if mode == RenderMode::Tui => {
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
            _ = ctrl_c(), if mode == RenderMode::Plain => {
                let _ = client.shutdown().await;
            }
        }
    }

    if mode == RenderMode::Tui {
        terminal_teardown();
    }
}
