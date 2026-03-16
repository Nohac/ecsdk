use bevy::app::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppRole {
    Server,
    Client,
}

pub trait RolePlugin {
    fn build_shared(&self, _app: &mut App) {}
    fn build_server(&self, _app: &mut App) {}
    fn build_client(&self, _app: &mut App) {}
}

pub trait AppRoleExt {
    fn add_shared_role_plugin<P: RolePlugin>(&mut self, plugin: P) -> &mut Self;
    fn add_role_plugin<P: RolePlugin>(&mut self, role: AppRole, plugin: P) -> &mut Self;
}

impl AppRoleExt for App {
    fn add_shared_role_plugin<P: RolePlugin>(&mut self, plugin: P) -> &mut Self {
        plugin.build_shared(self);
        self
    }

    fn add_role_plugin<P: RolePlugin>(&mut self, role: AppRole, plugin: P) -> &mut Self {
        plugin.build_shared(self);
        match role {
            AppRole::Server => plugin.build_server(self),
            AppRole::Client => plugin.build_client(self),
        }
        self
    }
}
