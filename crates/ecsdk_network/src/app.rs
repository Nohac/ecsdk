use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy_replicon::prelude::*;
use ecsdk_app::AsyncApp;
use ecsdk_core::ApplyMessage;
use serde::{Serialize, de::DeserializeOwned};

use crate::{ClientRepliconPlugin, ServerRepliconPlugin};

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

pub struct IsomorphicApp<M: ApplyMessage> {
    plugins: Vec<Box<dyn IsomorphicPlugin>>,
    marker: std::marker::PhantomData<M>,
}

impl<M: ApplyMessage> IsomorphicApp<M> {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            marker: std::marker::PhantomData,
        }
    }

    pub fn add_plugin<P: IsomorphicPlugin + 'static>(&mut self, plugin: P) -> &mut Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    pub fn build_server(self) -> AsyncApp<M> {
        self.build(AppRole::Server)
    }

    pub fn build_client(self) -> AsyncApp<M> {
        self.build(AppRole::Client)
    }

    fn build(self, role: AppRole) -> AsyncApp<M> {
        let mut app = ecsdk_app::setup::<M>();
        match role {
            AppRole::Server => app.add_plugins(ServerRepliconPlugin),
            AppRole::Client => app.add_plugins(ClientRepliconPlugin),
        };
        for plugin in self.plugins {
            plugin.build_shared(&mut app);
            match role {
                AppRole::Server => plugin.build_server(&mut app),
                AppRole::Client => plugin.build_client(&mut app),
            }
        }

        app
    }
}

impl<M: ApplyMessage> Default for IsomorphicApp<M> {
    fn default() -> Self {
        Self::new()
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

    fn add_isomorphic_plugin<P: IsomorphicPlugin>(
        &mut self,
        role: AppRole,
        plugin: P,
    ) -> &mut Self {
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

pub trait RequestPlugin: 'static {
    type Request: ClientRequest;
    type Trigger: Component;

    fn request() -> Self::Request
    where
        Self::Request: Default,
    {
        Default::default()
    }

    fn auto_register_shared() -> bool {
        true
    }

    fn auto_register_server() -> bool {
        true
    }

    fn auto_register_client() -> bool {
        true
    }

    fn register_shared(app: &mut App)
    where
        Self::Request: Default,
        for<'a> <Self::Request as Event>::Trigger<'a>: Default,
        for<'a> <<Self::Request as ClientRequest>::Response as Event>::Trigger<'a>: Default,
    {
        Self::Request::register(app);
    }

    fn register_server(app: &mut App) {
        Self::build_server(app);
    }

    fn register_client(app: &mut App)
    where
        Self::Request: Default,
        Self: Sized,
    {
        app.add_observer(send_request_on_trigger::<Self>);
        Self::build_client(app);
    }

    fn build_server(app: &mut App);
    fn build_client(app: &mut App);
}

impl<T> IsomorphicPlugin for T
where
    T: RequestPlugin + 'static,
    T::Request: Default,
    for<'a> <T::Request as Event>::Trigger<'a>: Default,
    for<'a> <<T::Request as ClientRequest>::Response as Event>::Trigger<'a>: Default,
{
    fn build_shared(&self, app: &mut App) {
        if T::auto_register_shared() {
            T::register_shared(app);
        }
    }

    fn build_server(&self, app: &mut App) {
        if T::auto_register_server() {
            T::register_server(app);
        }
    }

    fn build_client(&self, app: &mut App) {
        if T::auto_register_client() {
            T::register_client(app);
        }
    }
}

fn send_request_on_trigger<T>(_trigger: On<Add, T::Trigger>, mut commands: Commands)
where
    T: RequestPlugin,
    T::Request: Default,
{
    commands.client_trigger(T::request());
}
