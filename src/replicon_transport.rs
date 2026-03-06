use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy::state::prelude::*;
use bevy_replicon::prelude::*;
use facet::Facet;
use tokio::sync::mpsc;

use crate::container::*;
use crate::protocol::{LogEvent, ServerExitNotice, ShutdownRequest};

// ---------------------------------------------------------------------------
// Packet type — roam-serializable wrapper for replicon byte messages
// ---------------------------------------------------------------------------

#[derive(Facet)]
pub struct RepliconPacket {
    pub channel_id: u8,
    pub data: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Roam service — bidirectional streaming of replicon packets
// ---------------------------------------------------------------------------

#[roam::service]
pub trait RepliconTransport {
    /// Bidirectional replicon stream: server pushes to `to_client`, client pushes to `from_client`.
    async fn replicate(
        &self,
        to_client: roam::Tx<RepliconPacket>,
        from_client: roam::Rx<RepliconPacket>,
    );

    /// Health check.
    async fn ping(&self) -> Result<String, String>;
}

// ---------------------------------------------------------------------------
// Shared replication plugin — ensures identical registration order
// ---------------------------------------------------------------------------

pub struct SharedReplicationPlugin;

impl Plugin for SharedReplicationPlugin {
    fn build(&self, app: &mut App) {
        // Component replication (order must match on server and client)
        app.replicate::<ContainerName>();
        app.replicate::<ImageRef>();
        app.replicate::<StartOrder>();
        app.replicate::<ContainerPhase>();
        app.replicate::<DownloadProgress>();
        app.replicate::<SystemEntity>();

        // Server → client events
        app.add_mapped_server_event::<LogEvent>(Channel::Ordered);
        app.add_server_event::<ServerExitNotice>(Channel::Ordered);

        // Client → server events
        app.add_client_event::<ShutdownRequest>(Channel::Ordered);
    }
}

// ---------------------------------------------------------------------------
// Server-side bridge: mpsc channels between async roam I/O and ECS systems
// ---------------------------------------------------------------------------

use std::collections::HashMap;

pub struct ServerClientChannels {
    pub from_client_rx: mpsc::UnboundedReceiver<RepliconPacket>,
    pub to_client_tx: mpsc::UnboundedSender<RepliconPacket>,
}

#[derive(Resource, Default)]
pub struct ServerBridge {
    pub clients: HashMap<Entity, ServerClientChannels>,
}

pub struct RoamServerPlugin;

impl Plugin for RoamServerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerBridge>();
        app.add_systems(
            PreUpdate,
            (server_manage_state, server_receive_packets)
                .chain()
                .in_set(ServerSystems::ReceivePackets),
        );
        app.add_systems(
            PostUpdate,
            server_send_packets
                .in_set(ServerSystems::SendPackets),
        );
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

fn server_receive_packets(
    mut bridge: ResMut<ServerBridge>,
    mut messages: ResMut<ServerMessages>,
) {
    for (client_entity, channels) in &mut bridge.clients {
        while let Ok(packet) = channels.from_client_rx.try_recv() {
            messages.insert_received(
                *client_entity,
                packet.channel_id as usize,
                packet.data,
            );
        }
    }
}

fn server_send_packets(
    mut messages: ResMut<ServerMessages>,
    bridge: Res<ServerBridge>,
) {
    for (client_entity, channel_id, bytes) in messages.drain_sent() {
        if let Some(channels) = bridge.clients.get(&client_entity) {
            let _ = channels.to_client_tx.send(RepliconPacket {
                channel_id: channel_id as u8,
                data: bytes.to_vec(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Client-side bridge: mpsc channels between async roam I/O and ECS systems
// ---------------------------------------------------------------------------

#[derive(Resource)]
pub struct ClientBridge {
    pub from_server_rx: mpsc::UnboundedReceiver<RepliconPacket>,
    pub to_server_tx: mpsc::UnboundedSender<RepliconPacket>,
}

pub struct RoamClientPlugin;

impl Plugin for RoamClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (client_manage_state, client_receive_packets)
                .chain()
                .in_set(ClientSystems::ReceivePackets),
        );
        app.add_systems(
            PostUpdate,
            client_send_packets
                .in_set(ClientSystems::SendPackets),
        );
    }
}

fn client_manage_state(
    bridge: Option<Res<ClientBridge>>,
    state: Res<State<ClientState>>,
    mut next_state: ResMut<NextState<ClientState>>,
) {
    match (bridge.is_some(), state.get()) {
        (true, &ClientState::Disconnected) => next_state.set(ClientState::Connected),
        (false, &ClientState::Connected) => next_state.set(ClientState::Disconnected),
        _ => {}
    }
}

fn client_receive_packets(
    mut bridge: Option<ResMut<ClientBridge>>,
    mut messages: ResMut<ClientMessages>,
) {
    let Some(bridge) = bridge.as_mut() else {
        return;
    };
    while let Ok(packet) = bridge.from_server_rx.try_recv() {
        messages.insert_received(packet.channel_id as usize, packet.data);
    }
}

fn client_send_packets(
    mut messages: ResMut<ClientMessages>,
    bridge: Option<Res<ClientBridge>>,
) {
    let Some(bridge) = bridge.as_ref() else {
        return;
    };
    for (channel_id, bytes) in messages.drain_sent() {
        let _ = bridge.to_server_tx.send(RepliconPacket {
            channel_id: channel_id as u8,
            data: bytes.to_vec(),
        });
    }
}
