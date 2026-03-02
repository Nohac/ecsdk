use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy_ecs::prelude::*;
use roam_stream::{HandshakeConfig, accept};
use tokio::net::UnixListener;
use tokio::sync::broadcast;
use tokio::{select, signal::ctrl_c, sync::mpsc};

use crate::backend::ContainerRuntime;
use crate::backend_mock::MockBackend;
use crate::bridge::{AppExit, EventSender, TokioHandle, WorldCmd};
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
pub struct TrackedContainer {
    phase: ContainerPhase,
    downloaded: u64,
    log_count: usize,
}

/// ECS system that diffs container state each tick and broadcasts changes.
#[allow(clippy::type_complexity)]
pub fn broadcast_events(
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
    // Build snapshot and detect changes
    let mut snapshot = Vec::new();

    for (_entity, name, image, order, phase, progress, log_buf) in &containers {
        let id = name.0.clone();
        let (dl, total) = progress.map(|p| (p.downloaded, p.total)).unwrap_or((0, 0));

        snapshot.push(ContainerSnapshot {
            info: ContainerInfo {
                id: id.clone(),
                name: name.0.clone(),
                image: image.0.clone(),
                order: order.0,
            },
            phase: Phase::from(*phase),
            downloaded: dl,
            total,
        });

        if let Some(prev) = tracked.get(&id) {
            if prev.phase != *phase {
                let _ = broadcast.tx.send(DaemonEvent::PhaseChanged {
                    id: id.clone(),
                    phase: Phase::from(*phase),
                });
            }

            if let Some(prog) = progress
                && prog.downloaded != prev.downloaded
            {
                let _ = broadcast.tx.send(DaemonEvent::Progress {
                    id: id.clone(),
                    downloaded: prog.downloaded,
                    total: prog.total,
                });
            }

            if log_buf.lines.len() > prev.log_count {
                for line in &log_buf.lines[prev.log_count..] {
                    let _ = broadcast.tx.send(DaemonEvent::Log {
                        id: id.clone(),
                        text: line.text.clone(),
                    });
                }
            }
        } else {
            // New container — emit its current phase
            let _ = broadcast.tx.send(DaemonEvent::PhaseChanged {
                id: id.clone(),
                phase: Phase::from(*phase),
            });
        }

        tracked.insert(
            id,
            TrackedContainer {
                phase: *phase,
                downloaded: dl,
                log_count: log_buf.lines.len(),
            },
        );
    }

    // System entity logs
    for (_entity, name, log_buf) in &system_query {
        let id = name.0.clone();
        let prev_count = tracked.get(&id).map(|t| t.log_count).unwrap_or(0);
        if log_buf.lines.len() > prev_count {
            for line in &log_buf.lines[prev_count..] {
                let _ = broadcast.tx.send(DaemonEvent::Log {
                    id: id.clone(),
                    text: line.text.clone(),
                });
            }
        }
        tracked
            .entry(id)
            .and_modify(|t| t.log_count = log_buf.lines.len())
            .or_insert(TrackedContainer {
                phase: ContainerPhase::Pending,
                downloaded: 0,
                log_count: log_buf.lines.len(),
            });
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
    schedule.add_systems(broadcast_events);
    schedule
}

/// Roam service implementation for the daemon.
#[derive(Clone)]
struct ComposeDaemonImpl {
    event_tx: broadcast::Sender<DaemonEvent>,
    snapshot: SharedSnapshot,
    cmd_tx: EventSender,
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
    let event_sender = EventSender(tx);

    let mut world = World::new();
    world.insert_resource(event_sender.clone());
    world.insert_resource(TokioHandle(tokio::runtime::Handle::current()));
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
        cmd_tx: event_sender,
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
