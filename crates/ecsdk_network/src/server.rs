use std::collections::HashMap;

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy::state::prelude::*;
use bevy_replicon::prelude::*;
use ecsdk_tasks::SpawnTask;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::connection::{
    Connected, ConnectionStateEntity, InitialConnection, spawn_connection_state_entity,
    sync_connection_markers,
};
use crate::{RepliconPacket, run_bridge};

struct ServerClientChannels {
    from_client_rx: mpsc::UnboundedReceiver<RepliconPacket>,
    to_client_tx: mpsc::UnboundedSender<RepliconPacket>,
}

#[derive(Resource, Default)]
pub struct ServerBridge {
    clients: HashMap<Entity, ServerClientChannels>,
}

pub struct ServerTransportPlugin;
pub struct ServerRepliconPlugin;

impl Plugin for ServerRepliconPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.add_plugins(bevy::time::TimePlugin);
        app.add_plugins(RepliconPlugins.build().set(ServerPlugin::new(PostUpdate)));
        app.add_plugins(ServerTransportPlugin);
    }
}

impl Plugin for ServerTransportPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerBridge>();
        app.replicate::<ConnectionStateEntity>();
        app.replicate::<InitialConnection>();
        app.replicate::<Connected>();
        app.add_systems(Startup, spawn_connection_state_entity);
        app.add_systems(
            PreUpdate,
            (server_manage_state, server_receive_packets)
                .chain()
                .in_set(ServerSystems::ReceivePackets),
        );
        app.add_systems(Update, sync_connection_markers);
        app.add_systems(
            PostUpdate,
            server_send_packets.in_set(ServerSystems::SendPackets),
        );
    }
}

pub struct AcceptClientCmd<S> {
    pub stream: S,
}

impl<S> Command for AcceptClientCmd<S>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    fn apply(self, world: &mut World) {
        let (to_client_tx, to_client_rx) = mpsc::unbounded_channel::<RepliconPacket>();
        let (from_client_tx, from_client_rx) = mpsc::unbounded_channel::<RepliconPacket>();

        let mut com = world.commands();
        let mut client = com.spawn(ConnectedClient { max_size: 1200 });
        let client_id = client.id();

        let stream = self.stream;
        client.spawn_task(move |task| async move {
            let mut to_client_rx = to_client_rx;
            let wake = task.clone();
            run_bridge(stream, &mut to_client_rx, &from_client_tx, move || {
                wake.wake();
            })
            .await;

            let entity = task.entity();
            task.queue_cmd_wake(move |world: &mut World| {
                UnregisterClientCmd { entity }.apply(world);
            });
        });

        world.resource_mut::<ServerBridge>().clients.insert(
            client_id,
            ServerClientChannels {
                from_client_rx,
                to_client_tx,
            },
        );
    }
}

pub struct UnregisterClientCmd {
    pub entity: Entity,
}

impl Command for UnregisterClientCmd {
    fn apply(self, world: &mut World) {
        world
            .resource_mut::<ServerBridge>()
            .clients
            .remove(&self.entity);
        if world.get_entity(self.entity).is_ok() {
            world.despawn(self.entity);
        }
    }
}

fn server_manage_state(
    bridge: Res<ServerBridge>,
    state: Res<State<ServerState>>,
    mut next_state: ResMut<NextState<ServerState>>,
) {
    match (bridge.clients.is_empty(), state.get()) {
        (false, &ServerState::Stopped) => next_state.set(ServerState::Running),
        (true, &ServerState::Running) => next_state.set(ServerState::Stopped),
        _ => {}
    }
}

fn server_receive_packets(mut bridge: ResMut<ServerBridge>, mut messages: ResMut<ServerMessages>) {
    for (client_entity, channels) in &mut bridge.clients {
        while let Ok(packet) = channels.from_client_rx.try_recv() {
            messages.insert_received(*client_entity, packet.channel_id as usize, packet.data);
        }
    }
}

fn server_send_packets(mut messages: ResMut<ServerMessages>, bridge: Res<ServerBridge>) {
    for (client_entity, channel_id, bytes) in messages.drain_sent() {
        if let Some(channels) = bridge.clients.get(&client_entity) {
            let _ = channels.to_client_tx.send(RepliconPacket {
                channel_id: channel_id as u8,
                data: bytes.to_vec(),
            });
        }
    }
}
