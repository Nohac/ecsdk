use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy::state::prelude::*;
use bevy_replicon::prelude::*;
use ecsdk_app::Receivers;
use ecsdk_core::ApplyMessage;
use ecsdk_tasks::SpawnCmdTask;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

pub use ecsdk_replicon_transport::{RepliconPacket, run_bridge};

#[derive(Component, Serialize, Deserialize)]
pub struct ConnectionStateEntity;

#[derive(Component, Serialize, Deserialize)]
pub struct InitialConnection;

#[derive(Component, Serialize, Deserialize)]
pub struct Connected;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppRole {
    Server,
    Client,
}

pub trait IsomorphicPlugin {
    fn build_shared(&self, _app: &mut App) {}
    fn build_server(&self, _app: &mut App) {}
    fn build_client(&self, _app: &mut App) {}
}

pub trait ScopedIsomorphicPlugin<S>: 'static {
    fn shared_scope(&self) -> Option<S> {
        None
    }

    fn server_scope(&self) -> Option<S> {
        None
    }

    fn client_scope(&self) -> Option<S> {
        None
    }

    fn build_shared(&self, _app: &mut App) {}
    fn build_server(&self, _app: &mut App) {}
    fn build_client(&self, _app: &mut App) {}
}

enum PluginEntry<S> {
    Always(Box<dyn IsomorphicPlugin>),
    Scoped(Box<dyn ScopedIsomorphicPlugin<S>>),
}

pub struct IsomorphicApp<M: ApplyMessage, S = ()> {
    plugins: Vec<PluginEntry<S>>,
    marker: std::marker::PhantomData<M>,
}

pub struct BuiltIsomorphicApp<M: ApplyMessage> {
    app: App,
    receivers: Receivers<M>,
}

impl<M: ApplyMessage, S: Copy + Eq + 'static> IsomorphicApp<M, S> {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            marker: std::marker::PhantomData,
        }
    }

    pub fn add_plugin<P: IsomorphicPlugin + 'static>(&mut self, plugin: P) -> &mut Self {
        self.plugins.push(PluginEntry::Always(Box::new(plugin)));
        self
    }

    pub fn add_scoped_plugin<P: ScopedIsomorphicPlugin<S> + 'static>(&mut self, plugin: P) -> &mut Self {
        self.plugins.push(PluginEntry::Scoped(Box::new(plugin)));
        self
    }

    pub fn build_server(self, scope: S) -> BuiltIsomorphicApp<M> {
        self.build(AppRole::Server, scope)
    }

    pub fn build_client(self, scope: S) -> BuiltIsomorphicApp<M> {
        self.build(AppRole::Client, scope)
    }

    fn build(self, role: AppRole, scope: S) -> BuiltIsomorphicApp<M> {
        let (mut app, receivers) = ecsdk_app::setup::<M>();
        match role {
            AppRole::Server => app.add_plugins(ServerRepliconPlugin),
            AppRole::Client => app.add_plugins(ClientRepliconPlugin),
        };
        for plugin in self.plugins {
            match plugin {
                PluginEntry::Always(plugin) => {
                    plugin.build_shared(&mut app);
                    match role {
                        AppRole::Server => plugin.build_server(&mut app),
                        AppRole::Client => plugin.build_client(&mut app),
                    }
                }
                PluginEntry::Scoped(plugin) => {
                    if plugin.shared_scope().is_none_or(|s| s == scope) {
                        plugin.build_shared(&mut app);
                    }
                    match role {
                        AppRole::Server => {
                            if plugin.server_scope().is_none_or(|s| s == scope) {
                                plugin.build_server(&mut app);
                            }
                        }
                        AppRole::Client => {
                            if plugin.client_scope().is_none_or(|s| s == scope) {
                                plugin.build_client(&mut app);
                            }
                        }
                    }
                }
            }
        }

        BuiltIsomorphicApp { app, receivers }
    }
}

impl<M: ApplyMessage, S: Copy + Eq + 'static> Default for IsomorphicApp<M, S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: ApplyMessage> BuiltIsomorphicApp<M> {
    pub fn into_parts(self) -> (App, Receivers<M>) {
        (self.app, self.receivers)
    }

    pub async fn run(mut self) {
        ecsdk_app::run_async(&mut self.app, self.receivers).await;
    }
}

impl<M: ApplyMessage> Deref for BuiltIsomorphicApp<M> {
    type Target = App;

    fn deref(&self) -> &Self::Target {
        &self.app
    }
}

impl<M: ApplyMessage> DerefMut for BuiltIsomorphicApp<M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.app
    }
}

pub trait IsomorphicAppExt {
    fn add_shared_plugin<P: IsomorphicPlugin>(&mut self, plugin: P) -> &mut Self;
    fn add_isomorphic_plugin<P: IsomorphicPlugin>(&mut self, role: AppRole, plugin: P)
        -> &mut Self;
}

impl IsomorphicAppExt for App {
    fn add_shared_plugin<P: IsomorphicPlugin>(&mut self, plugin: P) -> &mut Self {
        plugin.build_shared(self);
        self
    }

    fn add_isomorphic_plugin<P: IsomorphicPlugin>(&mut self, role: AppRole, plugin: P) -> &mut Self {
        plugin.build_shared(self);
        match role {
            AppRole::Server => plugin.build_server(self),
            AppRole::Client => plugin.build_client(self),
        }
        self
    }
}

pub trait ClientRequest: Event + Serialize + DeserializeOwned {
    type Response: Event + Serialize + DeserializeOwned;

    const REQUEST_CHANNEL: Channel = Channel::Ordered;
    const RESPONSE_CHANNEL: Channel = Channel::Ordered;

    fn register(app: &mut App)
    where
        for<'a> <Self as Event>::Trigger<'a>: Default,
        for<'a> <Self::Response as Event>::Trigger<'a>: Default,
    {
        app.add_client_event::<Self>(Self::REQUEST_CHANNEL);
        app.add_server_event::<Self::Response>(Self::RESPONSE_CHANNEL);
    }

    fn response_mode(client_id: ClientId) -> SendMode {
        SendMode::Direct(client_id)
    }

    fn reply(commands: &mut Commands, client_id: ClientId, response: Self::Response) {
        commands.server_trigger(ToClients {
            mode: Self::response_mode(client_id),
            message: response,
        });
    }
}

pub trait RequestPlugin<S = ()> {
    type Request: ClientRequest;
    type Trigger: Component;

    fn request() -> Self::Request
    where
        Self::Request: Default,
    {
        Default::default()
    }

    fn shared_scope() -> Option<S> {
        None
    }

    fn server_scope() -> Option<S> {
        None
    }

    fn client_scope() -> Option<S> {
        None
    }

    fn build_server(app: &mut App);
    fn build_client(app: &mut App);
}

impl<T, S> ScopedIsomorphicPlugin<S> for T
where
    T: RequestPlugin<S> + 'static,
    S: Copy + 'static,
    T::Request: Default,
    for<'a> <T::Request as Event>::Trigger<'a>: Default,
    for<'a> <<T::Request as ClientRequest>::Response as Event>::Trigger<'a>: Default,
{
    fn shared_scope(&self) -> Option<S> {
        <T as RequestPlugin<S>>::shared_scope()
    }

    fn server_scope(&self) -> Option<S> {
        <T as RequestPlugin<S>>::server_scope()
    }

    fn client_scope(&self) -> Option<S> {
        <T as RequestPlugin<S>>::client_scope()
    }

    fn build_shared(&self, app: &mut App) {
        T::Request::register(app);
    }

    fn build_server(&self, app: &mut App) {
        T::build_server(app);
    }

    fn build_client(&self, app: &mut App) {
        app.add_observer(send_request_on_trigger::<T, S>);
        T::build_client(app);
    }
}

fn send_request_on_trigger<T, S>(_trigger: On<Add, T::Trigger>, mut commands: Commands)
where
    T: RequestPlugin<S>,
    T::Request: Default,
{
    commands.client_trigger(T::request());
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
pub struct ServerRepliconPlugin;
pub struct ClientRepliconPlugin;

impl Plugin for ServerRepliconPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.add_plugins(bevy::time::TimePlugin);
        app.add_plugins(RepliconPlugins.build().set(ServerPlugin::new(PostUpdate)));
        app.add_plugins(ServerTransportPlugin);
    }
}

impl Plugin for ClientRepliconPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::state::app::StatesPlugin);
        app.add_plugins(bevy::time::TimePlugin);
        app.add_plugins(RepliconPlugins);
        app.add_plugins(ClientTransportPlugin);
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
        app.add_systems(
            Update,
            sync_connection_markers,
        );
        app.add_systems(
            PostUpdate,
            server_send_packets.in_set(ServerSystems::SendPackets),
        );
    }
}

fn spawn_connection_state_entity(mut commands: Commands) {
    commands.spawn((ConnectionStateEntity, Replicated));
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

// ── ConnectClientCmd ──

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

fn sync_connection_markers(
    clients: Query<Entity, With<ConnectedClient>>,
    connection_state: Single<Entity, With<ConnectionStateEntity>>,
    connected: Query<(), With<Connected>>,
    initial: Query<(), With<InitialConnection>>,
    mut commands: Commands,
) {
    let entity = *connection_state;
    let has_clients = !clients.is_empty();
    let is_connected = connected.get(entity).is_ok();
    let has_initial = initial.get(entity).is_ok();

    if has_clients {
        if !has_initial {
            commands.entity(entity).insert(InitialConnection);
        }
        if !is_connected {
            commands.entity(entity).insert(Connected);
        }
    } else if is_connected {
        commands.entity(entity).remove::<Connected>();
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
