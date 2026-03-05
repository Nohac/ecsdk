use std::collections::HashMap;

use bevy_ecs::prelude::*;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use roam_stream::{HandshakeConfig, NoDispatcher, connect};
use tokio::sync::mpsc;
use tokio::{select, signal::ctrl_c};

use crate::app::TaskQueue;
use crate::backend::ContainerRuntime;
use crate::backend_mirror::MirrorBackend;
use crate::bridge::AppExit;
use crate::container::*;
use crate::ipc::{ComposeDaemonClient, DaemonConnector, SOCKET_PATH};
use crate::lifecycle::{Backend, LifecyclePlugin};
use crate::protocol::DaemonEvent;
use crate::render::{CrosstermPlugin, RenderMode};
use crate::task::CommandSender;

/// Run the client: connect to daemon, mirror its lifecycle locally, render.
pub async fn run_client(mode: RenderMode) {
    // Connect to daemon
    let connector = DaemonConnector::new(SOCKET_PATH);
    let roam_client = connect(connector, HandshakeConfig::default(), NoDispatcher);
    let client = ComposeDaemonClient::new(roam_client);

    // Set up event channel for subscribe stream
    let (event_tx, mut event_rx) = roam::channel::<DaemonEvent>();

    // Start subscription — polled in select loop, no spawn needed
    let subscribe_client = client.clone();
    let mut subscribe_fut = Box::pin(async move {
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
        _ = &mut subscribe_fut => {
            eprintln!("Subscribe ended before snapshot");
            return;
        }
    };

    let (mut app, cmd_rx) = crate::app::setup();

    let shutdown_client = client.clone();
    app.add_plugins(LifecyclePlugin);
    app.add_plugins(CrosstermPlugin::new(mode).on_event(move |event, _cmd| {
        let c = shutdown_client.clone();
        async move {
            if let Event::Key(key) = event
                && key.code == KeyCode::Char('c')
                && key.modifiers.contains(KeyModifiers::CONTROL)
            {
                let _ = c.shutdown().await;
            }
        }
    }));

    // Per-entity mpsc channels — forwarder demuxes daemon events by container ID.
    // Channels are created before run_async() so no events are missed.
    let mut event_channels: HashMap<String, mpsc::UnboundedSender<DaemonEvent>> = HashMap::new();

    // Terminal states (Running/Stopped/Failed) keep their phase so they don't
    // block ordering. Transitional states reset to Pending so the lifecycle
    // re-drives them through MirrorBackend (which catches remaining events).
    for c in &snapshot {
        let phase = match ContainerPhase::from(c.phase) {
            p @ (ContainerPhase::Running | ContainerPhase::Stopped | ContainerPhase::Failed) => p,
            _ => ContainerPhase::Pending,
        };
        let (tx, rx) = mpsc::unbounded_channel();
        event_channels.insert(c.info.name.clone(), tx);
        app.world_mut().spawn((
            ContainerName(c.info.name.clone()),
            ImageRef(c.info.image.clone()),
            StartOrder(c.info.order),
            phase,
            LogBuffer::default(),
            Backend(ContainerRuntime::from(MirrorBackend::new(rx))),
        ));
    }
    app.world_mut().spawn((
        ContainerName("[system]".into()),
        LogBuffer::default(),
        SystemEntity,
    ));

    let cmd_sender = app.world().resource::<CommandSender>().clone();

    // Daemon event forwarder — demuxes by container ID into per-entity channels
    let fwd_sender = cmd_sender.clone();
    {
        let mut tasks = app.world_mut().resource_mut::<TaskQueue>();
        tasks.push(async move {
            let mut subscribe_fut = subscribe_fut;
            loop {
                select! {
                    result = event_rx.recv() => {
                        match result.ok().flatten() {
                            Some(event) => {
                                if matches!(event, DaemonEvent::Exit) {
                                    fwd_sender.send(|w: &mut World| w.resource_mut::<AppExit>().0 = true);
                                }
                                match &event {
                                    DaemonEvent::PhaseChanged { id, .. }
                                    | DaemonEvent::Progress { id, .. }
                                    | DaemonEvent::Log { id, .. } => {
                                        if let Some(tx) = event_channels.get(id) {
                                            let _ = tx.send(event);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            None => {
                                fwd_sender.send(|w: &mut World| w.resource_mut::<AppExit>().0 = true);
                                break;
                            }
                        }
                    }
                    _ = &mut subscribe_fut => {
                        fwd_sender.send(|w: &mut World| w.resource_mut::<AppExit>().0 = true);
                        break;
                    }
                }
            }
        });

        // Ctrl+C — task, not spawn
        let shutdown_client = client.clone();
        tasks.push(async move {
            ctrl_c().await.ok();
            let _ = shutdown_client.shutdown().await;
        });
    }

    crate::app::run_async(app, cmd_rx).await;
}
