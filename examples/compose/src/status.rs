use std::time::SystemTime;

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy_replicon::prelude::*;
use ecsdk_core::AppExit;
use ecsdk_replicon::{ClientRequest, InitialConnection};
use serde::{Deserialize, Serialize};

use crate::role::RolePlugin;

#[derive(Event, Serialize, Deserialize)]
pub struct StatusRequest;

impl ClientRequest for StatusRequest {
    type Response = StatusResponse;
}

#[derive(Event, Serialize, Deserialize)]
pub struct StatusResponse {
    pub time: SystemTime,
    pub note: String,
}

pub struct StatusFeature;

impl RolePlugin for StatusFeature {
    fn build_shared(&self, app: &mut App) {
        StatusRequest::register(app);
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
    trigger: On<FromClient<StatusRequest>>,
    mut commands: Commands,
) {
    StatusRequest::reply(
        &mut commands,
        trigger.event().client_id,
        StatusResponse {
            time: SystemTime::now(),
            note: "hello from server".into(),
        },
    );
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
