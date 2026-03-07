use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy::state::prelude::*;
use bevy_replicon::prelude::*;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_util::bytes::Bytes;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::container::*;
use crate::protocol::{LogEvent, ServerExitNotice, ShutdownRequest};

// ---------------------------------------------------------------------------
// Packet type — [channel_id: u8][data...] inside a length-delimited frame
// ---------------------------------------------------------------------------

pub struct RepliconPacket {
    pub channel_id: u8,
    pub data: Vec<u8>,
}

impl RepliconPacket {
    fn encode(&self) -> Bytes {
        let mut buf = Vec::with_capacity(1 + self.data.len());
        buf.push(self.channel_id);
        buf.extend_from_slice(&self.data);
        buf.into()
    }

    fn decode(frame: Bytes) -> Option<Self> {
        if frame.is_empty() {
            return None;
        }
        Some(Self {
            channel_id: frame[0],
            data: frame[1..].to_vec(),
        })
    }
}

// ---------------------------------------------------------------------------
// Bidirectional bridge: framed stream ↔ mpsc channels
// ---------------------------------------------------------------------------

/// Run bidirectional forwarding between an async stream and mpsc channels.
/// Returns when either direction disconnects.
pub async fn run_bridge(
    stream: impl AsyncRead + AsyncWrite + Send + Unpin,
    to_remote_rx: &mut mpsc::UnboundedReceiver<RepliconPacket>,
    from_remote_tx: &mpsc::UnboundedSender<RepliconPacket>,
    wake: impl Fn(),
) {
    let (mut sink, mut source) = Framed::new(stream, LengthDelimitedCodec::new()).split();

    let send_to_remote = async {
        while let Some(packet) = to_remote_rx.recv().await {
            if sink.send(packet.encode()).await.is_err() {
                break;
            }
        }
    };

    let recv_from_remote = async {
        while let Some(Ok(frame)) = source.next().await {
            if let Some(packet) = RepliconPacket::decode(frame.into()) {
                let _ = from_remote_tx.send(packet);
                wake();
            }
        }
    };

    tokio::select! {
        _ = send_to_remote => {}
        _ = recv_from_remote => {}
    }
}

// ---------------------------------------------------------------------------
// Shared replication plugin — ensures identical registration order
// ---------------------------------------------------------------------------

pub struct SharedReplicationPlugin;

impl Plugin for SharedReplicationPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<ContainerName>();
        app.replicate::<ImageRef>();
        app.replicate::<StartOrder>();
        app.replicate::<ContainerPhase>();
        app.replicate::<DownloadProgress>();
        app.replicate::<SystemEntity>();

        app.add_mapped_server_event::<LogEvent>(Channel::Ordered);
        app.add_server_event::<ServerExitNotice>(Channel::Ordered);
        app.add_client_event::<ShutdownRequest>(Channel::Ordered);
    }
}

// ---------------------------------------------------------------------------
// Server-side bridge: mpsc channels between async I/O and ECS systems
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

pub struct ServerTransportPlugin;

impl Plugin for ServerTransportPlugin {
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
            server_send_packets.in_set(ServerSystems::SendPackets),
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
// Client-side bridge: mpsc channels between async I/O and ECS systems
// ---------------------------------------------------------------------------

#[derive(Resource)]
pub struct ClientBridge {
    pub from_server_rx: mpsc::UnboundedReceiver<RepliconPacket>,
    pub to_server_tx: mpsc::UnboundedSender<RepliconPacket>,
}

pub struct ClientTransportPlugin;

impl Plugin for ClientTransportPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (client_manage_state, client_receive_packets)
                .chain()
                .in_set(ClientSystems::ReceivePackets),
        );
        app.add_systems(
            PostUpdate,
            client_send_packets.in_set(ClientSystems::SendPackets),
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
