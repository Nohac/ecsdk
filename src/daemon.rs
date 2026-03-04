use std::collections::HashMap;

use bevy_ecs::prelude::*;
use roam_stream::{HandshakeConfig, accept};
use tokio::net::UnixListener;
use tokio::sync::broadcast;
use tokio::signal::ctrl_c;

use crate::app::App;
use crate::app::Update;
use crate::backend::ContainerRuntime;
use crate::backend_mock::MockBackend;
use crate::bridge::AppExit;
use crate::container::*;
use crate::ipc::{ComposeDaemon, ComposeDaemonDispatcher, SOCKET_PATH};
use crate::lifecycle::{Backend, LifecyclePlugin, ShutdownAll};
use crate::protocol::{ContainerInfo, ContainerSnapshot, DaemonEvent, Phase};
use crate::task::CommandSender;

/// Resource holding the broadcast channel for IPC events.
#[derive(Resource)]
pub struct IpcBroadcast {
    pub tx: broadcast::Sender<DaemonEvent>,
}

/// Tracked state per container for change detection.
struct TrackedContainer {
    phase: ContainerPhase,
    downloaded: u64,
    log_count: usize,
}

impl TrackedContainer {
    /// Diff current state against tracked, returning events for any changes.
    /// Updates tracked state in place.
    fn diff(
        &mut self,
        id: &str,
        phase: ContainerPhase,
        progress: Option<&DownloadProgress>,
        log_buf: &LogBuffer,
    ) -> Vec<DaemonEvent> {
        let mut events = Vec::new();

        if self.phase != phase {
            events.push(DaemonEvent::PhaseChanged {
                id: id.to_string(),
                phase: Phase::from(phase),
            });
            self.phase = phase;
        }

        if let Some(prog) = progress {
            if prog.downloaded != self.downloaded {
                events.push(DaemonEvent::Progress {
                    id: id.to_string(),
                    downloaded: prog.downloaded,
                    total: prog.total,
                });
            }
            self.downloaded = prog.downloaded;
        } else {
            self.downloaded = 0;
        }

        for line in &log_buf.lines[self.log_count..] {
            events.push(DaemonEvent::Log {
                id: id.to_string(),
                text: line.text.clone(),
            });
        }
        self.log_count = log_buf.lines.len();

        events
    }

    /// Diff logs only (for system entity).
    fn diff_logs(&mut self, id: &str, log_buf: &LogBuffer) -> Vec<DaemonEvent> {
        let mut events = Vec::new();
        for line in &log_buf.lines[self.log_count..] {
            events.push(DaemonEvent::Log {
                id: id.to_string(),
                text: line.text.clone(),
            });
        }
        self.log_count = log_buf.lines.len();
        events
    }
}

/// Build a full snapshot by querying the ECS world directly.
fn build_snapshot(world: &mut World) -> Vec<ContainerSnapshot> {
    let mut query = world.query_filtered::<(
        &ContainerName,
        &ImageRef,
        &StartOrder,
        &ContainerPhase,
        Option<&DownloadProgress>,
    ), Without<SystemEntity>>();

    query
        .iter(world)
        .map(|(name, image, order, phase, progress)| {
            let (dl, total) = progress.map(|p| (p.downloaded, p.total)).unwrap_or((0, 0));
            ContainerSnapshot {
                info: ContainerInfo {
                    id: name.0.clone(),
                    name: name.0.clone(),
                    image: image.0.clone(),
                    order: order.0,
                },
                phase: Phase::from(*phase),
                downloaded: dl,
                total,
            }
        })
        .collect()
}

/// ECS system that diffs container state each tick and broadcasts changes.
#[allow(clippy::type_complexity)]
fn broadcast_events(
    containers: Query<
        (
            &ContainerName,
            &ContainerPhase,
            Option<&DownloadProgress>,
            &LogBuffer,
        ),
        Without<SystemEntity>,
    >,
    system_query: Query<(&ContainerName, &LogBuffer), With<SystemEntity>>,
    broadcast: Res<IpcBroadcast>,
    exit: Res<AppExit>,
    mut tracked: Local<HashMap<String, TrackedContainer>>,
) {
    for (name, phase, progress, log_buf) in &containers {
        let id = name.0.clone();

        let events = if let Some(prev) = tracked.get_mut(&id) {
            prev.diff(&id, *phase, progress, log_buf)
        } else {
            tracked.insert(
                id.clone(),
                TrackedContainer {
                    phase: *phase,
                    downloaded: progress.map(|p| p.downloaded).unwrap_or(0),
                    log_count: log_buf.lines.len(),
                },
            );
            vec![DaemonEvent::PhaseChanged {
                id: id.clone(),
                phase: Phase::from(*phase),
            }]
        };

        for event in events {
            let _ = broadcast.tx.send(event);
        }
    }

    for (name, log_buf) in &system_query {
        let id = name.0.clone();
        let prev = tracked.entry(id.clone()).or_insert(TrackedContainer {
            phase: ContainerPhase::Pending,
            downloaded: 0,
            log_count: 0,
        });
        for event in prev.diff_logs(&id, log_buf) {
            let _ = broadcast.tx.send(event);
        }
    }

    if exit.0 {
        let _ = broadcast.tx.send(DaemonEvent::Exit);
    }
}

/// Roam service implementation for the daemon.
#[derive(Clone)]
struct ComposeDaemonImpl {
    event_tx: broadcast::Sender<DaemonEvent>,
    cmd_tx: CommandSender,
}

impl ComposeDaemon for ComposeDaemonImpl {
    async fn subscribe(&self, _cx: &roam::Context, events: roam::Tx<DaemonEvent>) {
        // Subscribe FIRST to ensure no events are missed between snapshot and streaming
        let mut rx = self.event_tx.subscribe();

        // Build snapshot on-demand from the ECS world
        let snapshot = self.cmd_tx.query(build_snapshot).await;
        if events.send(&DaemonEvent::Snapshot(snapshot)).await.is_err() {
            return;
        }

        // Forward all subsequent events
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if events.send(&event).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    async fn shutdown(&self, _cx: &roam::Context) -> Result<String, String> {
        self.cmd_tx.trigger(ShutdownAll);
        Ok("Shutdown initiated".into())
    }

    async fn ping(&self, _cx: &roam::Context) -> Result<String, String> {
        Ok("pong".into())
    }
}

/// Run the daemon process: ECS world + IPC server on a Unix socket.
pub async fn run_daemon() {
    // Clean up stale socket
    let _ = std::fs::remove_file(SOCKET_PATH);

    let (broadcast_tx, _) = broadcast::channel::<DaemonEvent>(256);

    let containers = [
        ("postgres", "postgres:16", 0),
        ("redis", "redis:7", 0),
        ("api-server", "myapp/api:latest", 1),
        ("web-frontend", "myapp/web:latest", 2),
    ];

    let mut app = App::new();
    app.add_plugin(LifecyclePlugin)
        .insert_resource(IpcBroadcast {
            tx: broadcast_tx.clone(),
        })
        .add_systems(
            Update,
            broadcast_events
                .after(crate::lifecycle::enforce_ordering)
                .after(crate::lifecycle::check_all_running)
                .after(crate::lifecycle::check_all_stopped),
        );

    for (name, image, order) in containers {
        app.world.spawn((
            ContainerName(name.into()),
            ImageRef(image.into()),
            StartOrder(order),
            ContainerPhase::Pending,
            LogBuffer::default(),
            Backend(ContainerRuntime::from(MockBackend::new(name, image))),
        ));
    }
    app.world.spawn((
        ContainerName("[system]".into()),
        LogBuffer::default(),
        SystemEntity,
    ));

    let cmd_sender = app.cmd_sender();
    let handler = ComposeDaemonImpl {
        event_tx: broadcast_tx,
        cmd_tx: cmd_sender.clone(),
    };

    // IPC listener — task, not spawn
    let listener = UnixListener::bind(SOCKET_PATH).expect("Failed to bind daemon socket");
    eprintln!("Daemon listening on {SOCKET_PATH}");

    let accept_handler = handler.clone();
    app.add_task(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let dispatcher = ComposeDaemonDispatcher::new(accept_handler.clone());
            // Per-connection handler IS genuine concurrency — spawn is correct here
            tokio::spawn(async move {
                match accept(stream, HandshakeConfig::default(), dispatcher).await {
                    Ok((_handle, _incoming, driver)) => {
                        let _ = driver.run().await;
                    }
                    Err(e) => eprintln!("Handshake failed: {e}"),
                }
            });
        }
    });

    // Ctrl+C — task, not spawn
    app.add_task(async move {
        ctrl_c().await.ok();
        cmd_sender.trigger(ShutdownAll);
    });

    app.run().await;

    // Cleanup
    let _ = std::fs::remove_file(SOCKET_PATH);
    eprintln!("Daemon shut down");
}
