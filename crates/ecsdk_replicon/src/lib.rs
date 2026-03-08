use std::collections::HashMap;

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy::state::prelude::*;
use bevy_replicon::prelude::*;
use ecsdk_tasks::SpawnCmdTask;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_util::bytes::Bytes;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

// ---------------------------------------------------------------------------
// Packet type — [channel_id: u8][data...] inside a length-delimited frame
// ---------------------------------------------------------------------------

pub struct RepliconPacket {
    pub channel_id: u8,
    pub data: Vec<u8>,
}

impl RepliconPacket {
    pub fn encode(&self) -> Bytes {
        let mut buf = Vec::with_capacity(1 + self.data.len());
        buf.push(self.channel_id);
        buf.extend_from_slice(&self.data);
        buf.into()
    }

    pub fn decode(frame: Bytes) -> Option<Self> {
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
// Server transport plugin
// ---------------------------------------------------------------------------

struct ServerClientChannels {
    from_client_rx: mpsc::UnboundedReceiver<RepliconPacket>,
    to_client_tx: mpsc::UnboundedSender<RepliconPacket>,
}

#[derive(Resource, Default)]
pub struct ServerBridge {
    clients: HashMap<Entity, ServerClientChannels>,
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

// ── Transport command structs ──

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
        client.spawn_cmd_task(move |client_cmd| async move {
            let mut to_client_rx = to_client_rx;
            let wake = client_cmd.clone();
            run_bridge(stream, &mut to_client_rx, &from_client_tx, move || {
                wake.wake();
            })
            .await;

            let entity = client_cmd.entity();
            client_cmd
                .send(move |world: &mut World| {
                    UnregisterClientCmd { entity }.apply(world);
                })
                .wake();
        });

        world.flush();

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

pub struct InsertClientBridgeCmd {
    pub from_server_rx: mpsc::UnboundedReceiver<RepliconPacket>,
    pub to_server_tx: mpsc::UnboundedSender<RepliconPacket>,
}

impl Command for InsertClientBridgeCmd {
    fn apply(self, world: &mut World) {
        world.insert_resource(ClientBridge {
            from_server_rx: self.from_server_rx,
            to_server_tx: self.to_server_tx,
        });
    }
}

pub struct RemoveClientBridgeCmd;

impl Command for RemoveClientBridgeCmd {
    fn apply(self, world: &mut World) {
        world.remove_resource::<ClientBridge>();
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

// ---------------------------------------------------------------------------
// Client transport plugin
// ---------------------------------------------------------------------------

#[derive(Resource)]
struct ClientBridge {
    from_server_rx: mpsc::UnboundedReceiver<RepliconPacket>,
    to_server_tx: mpsc::UnboundedSender<RepliconPacket>,
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

fn client_send_packets(mut messages: ResMut<ClientMessages>, bridge: Option<Res<ClientBridge>>) {
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
