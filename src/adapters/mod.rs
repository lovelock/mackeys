//! DE focus adapters — replaceable component (spec §4).
//! v1: `kwin` (KWin script + dbus) and `none` (no focus reports).
//! Placeholder trait skeleton; real registry wiring lands with the kwin
//! adapter (Task 7 of the core plan).

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowReport {
    pub caption: String,
    pub resource_class: String,
    pub resource_name: String,
}

pub trait FocusSource: Send + Sync {
    fn id(&self) -> &'static str;
    fn install(&self) -> anyhow::Result<()>;
    fn uninstall(&self) -> anyhow::Result<()>;
    fn current(&self) -> Option<WindowReport>;
    fn spawn_watch(
        self: std::sync::Arc<Self>,
        tx: std::sync::mpsc::Sender<Option<WindowReport>>,
    ) -> anyhow::Result<()>;
}
