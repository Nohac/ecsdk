use bevy::app::App;
use bevy::ecs::prelude::*;
use bevy_replicon::prelude::ClientTriggerExt;
use ecsdk_replicon::ClientRequest;

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

pub trait RequestPlugin {
    type Request: ClientRequest;
    type Trigger: Component;

    fn request() -> Self::Request
    where
        Self::Request: Default,
    {
        Default::default()
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
        T::Request::register(app);
    }

    fn build_server(&self, app: &mut App) {
        T::build_server(app);
    }

    fn build_client(&self, app: &mut App) {
        app.add_observer(send_request_on_trigger::<T>);
        T::build_client(app);
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

fn send_request_on_trigger<T>(_trigger: On<Add, T::Trigger>, mut commands: Commands)
where
    T: RequestPlugin,
    T::Request: Default,
{
    commands.client_trigger(T::request());
}
