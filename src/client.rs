use std::collections::HashMap;

use bevy_ecs::prelude::*;
use crossterm::event::{Event, KeyCode, KeyModifiers};
use roam_stream::{HandshakeConfig, NoDispatcher, connect};
use tokio::sync::broadcast;
use tokio::{select, signal::ctrl_c};

use crate::app::App;
use crate::backend::ContainerRuntime;
use crate::backend_mirror::MirrorBackend;
use crate::bridge::AppExit;
use crate::container::*;
use crate::ipc::{ComposeDaemonClient, DaemonConnector, SOCKET_PATH};
use crate::lifecycle::LifecyclePlugin;
use crate::protocol::DaemonEvent;
use crate::render::{CrosstermPlugin, RenderMode};

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

    let mut app = App::new();

    let shutdown_client = client.clone();
    app.add_plugin(LifecyclePlugin {
        backend: ContainerRuntime::from(mirror),
    })
    .add_plugin(CrosstermPlugin::new(mode).on_event(move |event, _cmd| {
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

    // Spawn entities from snapshot
    for c in &snapshot {
        app.world.spawn((
            ContainerName(c.info.name.clone()),
            ImageRef(c.info.image.clone()),
            StartOrder(c.info.order),
            ContainerPhase::Pending,
            LogBuffer::default(),
        ));
    }
    app.world.spawn((
        ContainerName("[system]".to_string()),
        LogBuffer::default(),
        SystemEntity,
    ));

    let cmd_sender = app.cmd_sender();

    // Daemon event forwarder — task, not spawn
    let fwd_sender = cmd_sender.clone();
    app.add_task(async move {
        let mut subscribe_fut = subscribe_fut;
        loop {
            select! {
                result = event_rx.recv() => {
                    match result.ok().flatten() {
                        Some(event) => {
                            if matches!(event, DaemonEvent::Exit) {
                                fwd_sender.send(|w: &mut World| w.resource_mut::<AppExit>().0 = true);
                            }
                            let _ = broadcast_tx.send(event);
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
    app.add_task(async move {
        ctrl_c().await.ok();
        let _ = shutdown_client.shutdown().await;
    });

    app.run().await;
}
