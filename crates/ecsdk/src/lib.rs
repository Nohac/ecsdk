pub use bevy;
#[cfg(feature = "replicon")]
pub use bevy_replicon;
#[cfg(feature = "app")]
pub use ecsdk_app as app;
pub use ecsdk_core as core;
#[cfg(feature = "macros")]
pub use ecsdk_macros as macros;
#[cfg(feature = "replicon")]
pub use ecsdk_replicon as replicon;
#[cfg(feature = "replicon")]
pub use ecsdk_replicon_transport as replicon_transport;
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

    pub use ecsdk_core::{AppExit, ApplyMessage, MessageQueue, ScheduleControl, WakeSignal};

    #[cfg(feature = "app")]
    pub use ecsdk_app::{Receivers, run_async, setup};

    #[cfg(feature = "macros")]
    pub use ecsdk_macros::{ClientRequest, StateComponent};

    #[cfg(feature = "replicon")]
    pub use bevy_replicon::prelude::*;

    #[cfg(feature = "replicon")]
    pub use ecsdk_replicon::{
        AppRole, ClientRequest, InitialConnection, IsomorphicAppExt, IsomorphicPlugin,
        RequestPlugin,
    };
}
