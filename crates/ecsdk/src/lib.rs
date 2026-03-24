pub use bevy;
#[cfg(feature = "network")]
pub use bevy_replicon;
#[cfg(feature = "app")]
pub use ecsdk_app as app;
pub use ecsdk_core as core;
#[cfg(feature = "macros")]
pub use ecsdk_macros as macros;
#[cfg(feature = "network")]
pub use ecsdk_network as network;
#[cfg(feature = "network")]
pub use ecsdk_transport as transport;
#[cfg(feature = "tasks")]
pub use ecsdk_tasks as tasks;
#[cfg(feature = "term")]
pub use ecsdk_term as term;
#[cfg(feature = "tracing")]
pub use ecsdk_tracing as tracing;
pub use serde;

pub mod prelude {
    pub use bevy::app::prelude::*;
    pub use bevy::ecs::prelude::*;

    pub use ecsdk_core::{
        ApplyMessage, MessageQueue, QueueCmdExt, ScheduleControl, SendMsgExt, WakeSignal,
    };

    #[cfg(feature = "app")]
    pub use ecsdk_app::{
        AppQueueCmdExt, AppSendMsgExt, AsyncApp, Receivers, RuntimeConfig, run_async, setup,
    };

    #[cfg(feature = "macros")]
    pub use ecsdk_macros::{ClientRequest, StateComponent};

    #[cfg(feature = "network")]
    pub use bevy_replicon::prelude::*;

    #[cfg(feature = "network")]
    pub use ecsdk_network::{
        AppRole, ClientRequest, InitialConnection, IsomorphicApp,
        IsomorphicAppExt, IsomorphicPlugin, RequestPlugin,
        ServerDisconnected,
    };
}
