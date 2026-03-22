use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy::state::prelude::*;
use bevy_replicon::prelude::*;
use ecsdk_tasks::SpawnCmdTask;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::connection::{Connected, ConnectionStateEntity, InitialConnection};
use crate::{RepliconPacket, run_bridge};

#[derive(Resource)]
struct ClientBridge {
    from_server_rx: mpsc::UnboundedReceiver<RepliconPacket>,
    to_server_tx: mpsc::UnboundedSender<RepliconPacket>,
}

pub struct ClientTransportPlugin;
pub struct ClientRepliconPlugin;

impl Plugin for ClientRepliconPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.add_plugins(bevy::time::TimePlugin);
        app.add_plugins(RepliconPlugins);
        app.add_plugins(ClientTransportPlugin);
    }
}

impl Plugin for ClientTransportPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<ConnectionStateEntity>();
        app.replicate::<InitialConnection>();
        app.replicate::<Connected>();
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

pub struct ConnectClientCmd<S> {
    pub stream: S,
}

impl<S> Command for ConnectClientCmd<S>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    fn apply(self, world: &mut World) {
        let (to_server_tx, to_server_rx) = mpsc::unbounded_channel::<RepliconPacket>();
        let (from_server_tx, from_server_rx) = mpsc::unbounded_channel::<RepliconPacket>();

        InsertClientBridgeCmd {
            from_server_rx,
            to_server_tx,
        }
        .apply(world);

        let mut com = world.commands();
        com.spawn_empty().spawn_cmd_task(move |cmd| async move {
            let mut to_server_rx = to_server_rx;
            let wake = cmd.clone();
            run_bridge(self.stream, &mut to_server_rx, &from_server_tx, move || {
                wake.wake();
            })
            .await;

            cmd.send(|world: &mut World| {
                RemoveClientBridgeCmd.apply(world);
            })
            .wake();
        });

        world.flush();
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
