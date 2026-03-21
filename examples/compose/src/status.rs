use std::time::SystemTime;

use ecsdk::prelude::*;
use serde::{Deserialize, Serialize};

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
    type Trigger = ecsdk::network::InitialConnection;

    fn auto_register_client() -> bool {
        false
    }

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
