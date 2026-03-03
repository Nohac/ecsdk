use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy_ecs::prelude::*;
use roam_stream::{HandshakeConfig, accept};
use tokio::net::UnixListener;
use tokio::sync::broadcast;
use tokio::{select, signal::ctrl_c, sync::mpsc};

use crate::backend::ContainerRuntime;
use crate::backend_mock::MockBackend;
use crate::bridge::{AppExit, WorldCmd};
use crate::task::CommandSender;
use crate::container::*;
use crate::ipc::{ComposeDaemon, ComposeDaemonDispatcher, SOCKET_PATH};
use crate::lifecycle::{Backend, ShutdownAll, register_observers};
use crate::protocol::{ContainerInfo, ContainerSnapshot, DaemonEvent, Phase};

/// Shared snapshot state readable by subscribe handlers.
#[derive(Clone, Default)]
pub struct SharedSnapshot(pub Arc<Mutex<Vec<ContainerSnapshot>>>);

/// Resource holding the broadcast channel for IPC events.
#[derive(Resource)]
pub struct IpcBroadcast {
    pub tx: broadcast::Sender<DaemonEvent>,
    pub snapshot: SharedSnapshot,
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

fn build_container_snapshot(
    name: &str,
    image: &str,
    order: u32,
    phase: ContainerPhase,
    progress: Option<&DownloadProgress>,
) -> ContainerSnapshot {
    let (dl, total) = progress.map(|p| (p.downloaded, p.total)).unwrap_or((0, 0));
    ContainerSnapshot {
        info: ContainerInfo {
            id: name.to_string(),
            name: name.to_string(),
            image: image.to_string(),
            order,
        },
        phase: Phase::from(phase),
        downloaded: dl,
        total,
    }
}

/// ECS system that diffs container state each tick and broadcasts changes.
#[allow(clippy::type_complexity)]
fn broadcast_events(
    containers: Query<
        (
            Entity,
            &ContainerName,
            &ImageRef,
            &StartOrder,
            &ContainerPhase,
            Option<&DownloadProgress>,
            &LogBuffer,
        ),
        Without<SystemEntity>,
    >,
    system_query: Query<(Entity, &ContainerName, &LogBuffer), With<SystemEntity>>,
    broadcast: Res<IpcBroadcast>,
    exit: Res<AppExit>,
    mut tracked: Local<HashMap<String, TrackedContainer>>,
) {
    let mut snapshot = Vec::new();

    for (_entity, name, image, order, phase, progress, log_buf) in &containers {
        let id = name.0.clone();

        snapshot.push(build_container_snapshot(
            &name.0, &image.0, order.0, *phase, progress,
        ));

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

    for (_entity, name, log_buf) in &system_query {
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

    // Update shared snapshot
    *broadcast.snapshot.0.lock().unwrap() = snapshot;

    // Signal exit to clients
    if exit.0 {
        let _ = broadcast.tx.send(DaemonEvent::Exit);
    }
}

/// Build the daemon's update schedule (enforce_ordering + broadcast).
fn build_daemon_update_schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.add_systems(crate::lifecycle::enforce_ordering.before(broadcast_events));
    schedule.add_systems(crate::lifecycle::check_all_running.before(broadcast_events));
    schedule.add_systems(crate::lifecycle::check_all_stopped.before(broadcast_events));
    schedule.add_systems(broadcast_events);
    schedule
}

/// Roam service implementation for the daemon.
#[derive(Clone)]
struct ComposeDaemonImpl {
    event_tx: broadcast::Sender<DaemonEvent>,
    snapshot: SharedSnapshot,
    cmd_tx: CommandSender,
}

impl ComposeDaemon for ComposeDaemonImpl {
    async fn subscribe(&self, _cx: &roam::Context, events: roam::Tx<DaemonEvent>) {
        // Subscribe FIRST to ensure no events are missed between snapshot and streaming
        let mut rx = self.event_tx.subscribe();

        // Send current snapshot
        let snapshot = self.snapshot.0.lock().unwrap().clone();
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
    let snapshot = SharedSnapshot::default();

    let (tx, mut rx) = mpsc::unbounded_channel::<WorldCmd>();
    let cmd_sender = CommandSender::new(tx, tokio::runtime::Handle::current());

    let mut world = World::new();
    world.insert_resource(cmd_sender.clone());
    world.init_resource::<AppExit>();
    world.insert_resource(Backend(ContainerRuntime::from(MockBackend)));
    world.init_resource::<MergedLogView>();
    world.insert_resource(IpcBroadcast {
        tx: broadcast_tx.clone(),
        snapshot: snapshot.clone(),
    });

    register_observers(&mut world);

    let mut startup = build_startup_schedule();
    let mut update = build_daemon_update_schedule();

    startup.run(&mut world);
    update.run(&mut world);

    // Start serving on Unix socket
    let listener = UnixListener::bind(SOCKET_PATH).expect("Failed to bind daemon socket");
    eprintln!("Daemon listening on {SOCKET_PATH}");

    let handler = ComposeDaemonImpl {
        event_tx: broadcast_tx,
        snapshot,
        cmd_tx: cmd_sender,
    };

    loop {
        select! {
            Some(cmd) = rx.recv() => {
                cmd(&mut world);
                update.run(&mut world);
                if world.resource::<AppExit>().0 { break; }
            }
            Ok((stream, _)) = listener.accept() => {
                let dispatcher = ComposeDaemonDispatcher::new(handler.clone());
                tokio::spawn(async move {
                    match accept(stream, HandshakeConfig::default(), dispatcher).await {
                        Ok((_handle, _incoming, driver)) => {
                            let _ = driver.run().await;
                        }
                        Err(e) => eprintln!("Handshake failed: {e}"),
                    }
                });
            }
            _ = ctrl_c() => {
                handler.cmd_tx.trigger(ShutdownAll);
            }
        }
    }

    // Cleanup
    let _ = std::fs::remove_file(SOCKET_PATH);
    eprintln!("Daemon shut down");
}
