use std::time::SystemTime;

use bevy::app::prelude::*;
use bevy::ecs::prelude::*;
use bevy_replicon::prelude::*;
use ecsdk_core::AppExit;
use ecsdk_macros::ClientRequest;
use serde::{Deserialize, Serialize};

use crate::isomorphic::RequestPlugin;

#[derive(Default, Event, ClientRequest, Serialize, Deserialize)]
#[request(response = "StatusResponse")]
pub struct StatusRequest;

#[derive(Event, Serialize, Deserialize)]
pub struct StatusResponse {
    pub time: SystemTime,
    pub note: String,
}

pub struct StatusFeature;

impl RequestPlugin for StatusFeature {
    type Request = StatusRequest;
    type Trigger = ecsdk_replicon::InitialConnection;

    fn build_server(app: &mut App) {
        app.add_observer(handle_status_request);
    }

    fn build_client(app: &mut App) {
        app.add_observer(handle_status_response);
    }
}

fn handle_status_request(trigger: On<FromClient<StatusRequest>>, mut commands: Commands) {
    StatusRequest::reply(
        &mut commands,
        trigger.event().client_id,
        StatusResponse {
            time: SystemTime::now(),
            note: "hello from server".into(),
        },
    );
}

fn handle_status_response(trigger: On<StatusResponse>, mut exit: ResMut<AppExit>) {
    let e = trigger.event();
    println!("time: {:?}", e.time);
    println!("note: {}", e.note);
    exit.0 = true;
}
