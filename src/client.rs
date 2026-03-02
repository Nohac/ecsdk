use std::collections::HashMap;

use bevy_ecs::prelude::*;
use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use futures_util::StreamExt;
use roam_stream::{HandshakeConfig, NoDispatcher, connect};
use tokio::{select, signal::ctrl_c, sync::mpsc};

use crate::bridge::{AppExit, EventSender, WorldCmd};
use crate::container::*;
use crate::ipc::{ComposeDaemonClient, DaemonConnector, SOCKET_PATH};
use crate::protocol::{ContainerId, DaemonEvent};
use crate::render::{
    RenderMode, TerminalSize, build_render_schedule, install_panic_hook, terminal_init,
    terminal_teardown,
};

/// Apply a daemon event to the local ECS world.
fn apply_daemon_event(
    world: &mut World,
    id_map: &mut HashMap<ContainerId, Entity>,
    event: DaemonEvent,
) {
    match event {
        DaemonEvent::Snapshot(containers) => {
            for c in containers {
                let phase: ContainerPhase = c.phase.into();
                let mut entity_cmd = world.spawn((
                    ContainerName(c.info.name.clone()),
                    ImageRef(c.info.image),
                    StartOrder(c.info.order),
                    phase,
                    LogBuffer::default(),
                ));
                if c.total > 0 {
                    entity_cmd.insert(DownloadProgress {
                        downloaded: c.downloaded,
                        total: c.total,
                    });
                }
                let e = entity_cmd.id();
                id_map.insert(c.info.id, e);
            }
            // System entity for global messages
            let sys = world
                .spawn((
                    ContainerName("[system]".to_string()),
                    LogBuffer::default(),
                    SystemEntity,
                ))
                .id();
            id_map.insert("[system]".to_string(), sys);
        }
        DaemonEvent::PhaseChanged { id, phase } => {
            if let Some(&entity) = id_map.get(&id) {
                let cp: ContainerPhase = phase.into();
                world.entity_mut(entity).insert(cp);
            }
        }
        DaemonEvent::Progress {
            id,
            downloaded,
            total,
        } => {
            if let Some(&entity) = id_map.get(&id) {
                if let Some(mut dp) = world.get_mut::<DownloadProgress>(entity) {
                    dp.downloaded = downloaded;
                    dp.total = total;
                } else {
                    world
                        .entity_mut(entity)
                        .insert(DownloadProgress { downloaded, total });
                }
            }
        }
        DaemonEvent::Log { id, text } => {
            if let Some(&entity) = id_map.get(&id)
                && let Some(mut log_buf) = world.get_mut::<LogBuffer>(entity)
            {
                log_buf.push(text);
            }
        }
        DaemonEvent::Exit => {
            world.resource_mut::<AppExit>().0 = true;
        }
    }
}

/// Run the client: connect to daemon, subscribe to events, render locally.
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

    // Fire off the subscribe call (runs until stream ends)
    let subscribe_client = client.clone();
    tokio::spawn(async move {
        let _ = subscribe_client.subscribe(event_tx).await;
    });

    // Local ECS world — renderers only, no lifecycle
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<WorldCmd>();

    let mut world = World::new();
    world.insert_resource(EventSender(cmd_tx));
    world.insert_resource(TerminalSize::query_now());
    world.init_resource::<AppExit>();
    world.init_resource::<MergedLogView>();

    let mut render = build_render_schedule(mode);
    let mut id_map: HashMap<ContainerId, Entity> = HashMap::new();

    let mut term_events = EventStream::new();

    loop {
        select! {
            // Arm 1: IPC events from daemon
            result = event_rx.recv() => {
                match result {
                    Ok(Some(event)) => {
                        apply_daemon_event(&mut world, &mut id_map, event);
                        world.flush();
                        render.run(&mut world);
                        if world.resource::<AppExit>().0 { break; }
                    }
                    // Stream ended — daemon disconnected
                    _ => break,
                }
            }
            // Arm 2: Local WorldCmd (e.g. terminal resize)
            Some(cmd) = cmd_rx.recv() => {
                cmd(&mut world);
                render.run(&mut world);
                if world.resource::<AppExit>().0 { break; }
            }
            // Arm 3: Terminal events (TUI mode — raw mode captures Ctrl+C as key event)
            Some(Ok(event)) = term_events.next(), if mode == RenderMode::Tui => {
                match event {
                    Event::Resize(cols, rows) => {
                        world.resource_mut::<TerminalSize>().update(cols, rows);
                        render.run(&mut world);
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
            // Arm 4: Ctrl+C (plain mode — SIGINT delivered normally)
            _ = ctrl_c(), if mode == RenderMode::Plain => {
                let _ = client.shutdown().await;
            }
        }
    }

    if mode == RenderMode::Tui {
        terminal_teardown();
    }
}
