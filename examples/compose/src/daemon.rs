use std::collections::HashMap;

use ecsdk::core::{AppExit, MessageQueue};
use ecsdk::prelude::*;
use ecsdk::tasks::SpawnTask;
use tokio::signal::ctrl_c;

use crate::backend_mock::MockBackend;
use crate::container::*;
use crate::lifecycle::*;
use crate::message::Message;

const LOG_ENTRY_CAP: usize = 200;

fn sync_log_entries(
    mut commands: Commands,
    query: Query<(Entity, &ContainerName, Ref<LogBuffer>), Changed<LogBuffer>>,
    related: Query<&LogView>,
    log_view: Query<Entity, With<LogView>>,
    mut tracked: Local<HashMap<Entity, usize>>,
    mut color_map: Local<HashMap<Entity, u8>>,
    mut next_sequence: Local<u64>,
    mut next_color_idx: Local<u8>,
) {
    let Ok(log_view) = log_view.single() else {
        return;
    };

    let mut appended = false;

    for (source, name, log_buf) in &query {
        let color_idx = *color_map.entry(source).or_insert_with(|| {
            let idx = *next_color_idx;
            *next_color_idx = next_color_idx.wrapping_add(1);
            idx
        });
        let sent = tracked.get(&source).copied().unwrap_or(0);
        for line in &log_buf.lines[sent..] {
            appended = true;
            *next_sequence += 1;
            commands.spawn((
                Replicated,
                LogEntry {
                    target: log_view,
                    sequence: *next_sequence,
                    label: name.0.clone(),
                    color_idx,
                    message: line.text.clone(),
                },
            ));
        }
        tracked.insert(source, log_buf.lines.len());
    }

    if appended && let Ok(entries) = related.get(log_view) {
        let excess = entries.len().saturating_sub(LOG_ENTRY_CAP);
        for entry in entries.iter().take(excess) {
            commands.entity(entry).despawn();
        }
    }
}

fn send_exit_notice(mut commands: Commands, exit: Res<AppExit>, mut sent: Local<bool>) {
    if exit.0 && !*sent {
        commands.server_trigger(ToClients {
            mode: SendMode::Broadcast,
            message: crate::protocol::ServerExitNotice,
        });
        *sent = true;
    }
}

fn handle_shutdown_request(
    _trigger: On<FromClient<crate::protocol::ShutdownRequest>>,
    mut commands: Commands,
) {
    commands.trigger(ShutdownAll);
}

// ---------------------------------------------------------------------------
// DaemonPlugin — bundles all server-side registration
// ---------------------------------------------------------------------------

pub struct ComposeServerPlugin;

impl Plugin for ComposeServerPlugin {
    fn build(&self, app: &mut App) {
        // Lifecycle + replicated logs/exit broadcast
        app.add_plugins(LifecyclePlugin);
        app.add_systems(
            PreUpdate,
            crate::container::drain_tracing_logs
                .run_if(resource_exists::<ecsdk::tracing::TracingReceiver>),
        );
        app.add_systems(Update, (sync_log_entries, send_exit_notice));
        app.add_observer(handle_shutdown_request);

        // Ctrl+C triggers graceful shutdown
        app.add_systems(Startup, spawn_ctrl_c_handler);

        let containers = [
            ("postgres", "postgres:16", 0),
            ("redis", "redis:7", 0),
            ("api-server", "myapp/api:latest", 1),
            ("web-frontend", "myapp/web:latest", 2),
        ];

        let state_queue = app.world().resource::<MessageQueue<Message>>().clone();
        for (name, image, order) in containers {
            state_queue.send(Message::SpawnContainer {
                name: name.into(),
                image: image.into(),
                start_order: order,
            });
        }

        let mut orchestrator = app.world_mut().spawn((build_orchestrator_sm(),));
        OrchestratorPhase::Deploying.insert_marker_world(&mut orchestrator);

        app.world_mut().spawn((
            ContainerName("[system]".into()),
            LogBuffer::default(),
            SystemEntity,
            Replicated,
        ));
        app.world_mut().spawn((LogView::default(), Replicated));

        // Backend factory — attach backends to containers spawned by state events
        app.add_observer(attach_mock_backend);
    }
}

fn spawn_ctrl_c_handler(mut commands: Commands) {
    commands.spawn_empty().spawn_task(move |cmd| async move {
        ctrl_c().await.ok();
        cmd.send_state(Message::RequestShutdown);
    });
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run_daemon(mut app: AsyncApp<Message>) -> AsyncApp<Message> {
    app.add_plugins(ComposeServerPlugin);
    app
}

fn attach_mock_backend(
    trigger: On<Add, ContainerName>,
    mut commands: Commands,
    query: Query<(&ContainerName, &ImageRef), Without<Backend>>,
) {
    let entity = trigger.event_target();
    let Ok((name, image)) = query.get(entity) else {
        return;
    };
    commands
        .entity(entity)
        .insert((Backend(MockBackend::new(&name.0, &image.0)), Replicated));
}
