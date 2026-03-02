use facet::Facet;

/// Stable container identity across processes (Bevy Entity is process-local).
pub type ContainerId = String;

#[derive(Debug, Clone, Facet)]
pub struct ContainerInfo {
    pub id: ContainerId,
    pub name: String,
    pub image: String,
    pub order: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum Phase {
    Pending,
    PullingImage,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum DaemonEvent {
    /// Initial state: full list of containers with current phases.
    Snapshot(Vec<ContainerSnapshot>),
    /// Phase transition for a container.
    PhaseChanged { id: ContainerId, phase: Phase },
    /// Download progress update.
    Progress {
        id: ContainerId,
        downloaded: u64,
        total: u64,
    },
    /// Log line from a container or system entity.
    Log { id: ContainerId, text: String },
    /// Daemon is exiting — client should shut down.
    Exit,
}

/// Per-container state sent in the initial snapshot.
#[derive(Debug, Clone, Facet)]
pub struct ContainerSnapshot {
    pub info: ContainerInfo,
    pub phase: Phase,
    pub downloaded: u64,
    pub total: u64,
}
