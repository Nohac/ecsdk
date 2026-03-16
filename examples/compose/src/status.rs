use std::time::SystemTime;

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy_replicon::prelude::*;
use ecsdk_core::AppExit;
use ecsdk_replicon::InitialConnection;
use serde::{Deserialize, Serialize};

use crate::role::RolePlugin;

#[derive(Event, Serialize, Deserialize)]
pub struct StatusRequest;

#[derive(Event, Serialize, Deserialize)]
pub struct StatusResponse {
    pub time: SystemTime,
    pub note: String,
}

pub struct StatusFeature;

impl RolePlugin for StatusFeature {
    fn build_shared(&self, app: &mut App) {
        app.add_server_event::<StatusResponse>(Channel::Ordered);
        app.add_client_event::<StatusRequest>(Channel::Ordered);
    }

    fn build_server(&self, app: &mut App) {
        app.add_observer(handle_status_request);
    }

    fn build_client(&self, app: &mut App) {
        app.add_observer(send_status_request_on_initial_connection);
        app.add_observer(handle_status_response);
    }
}

fn handle_status_request(
    _trigger: On<FromClient<StatusRequest>>,
    mut commands: Commands,
) {
    commands.server_trigger(ToClients {
        mode: SendMode::Broadcast,
        message: StatusResponse {
            time: SystemTime::now(),
            note: "hello from server".into(),
        },
    });
}

fn send_status_request_on_initial_connection(
    _trigger: On<Add, InitialConnection>,
    mut commands: Commands,
) {
    commands.client_trigger(StatusRequest);
}

fn handle_status_response(trigger: On<StatusResponse>, mut exit: ResMut<AppExit>) {
    let e = trigger.event();
    println!("time: {:?}", e.time);
    println!("note: {}", e.note);
    exit.0 = true;
}
