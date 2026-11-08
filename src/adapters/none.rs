//! `none` adapter: no focus reports; contexts stay Gui unless config forces.

use super::WindowReport;
use std::sync::mpsc::SyncSender;

#[derive(Default)]
pub struct NoneAdapter;

impl super::FocusSource for NoneAdapter {
    fn id(&self) -> &'static str {
        "none"
    }
    fn install(&self) -> anyhow::Result<()> {
        tracing::info!("none adapter: nothing to install");
        Ok(())
    }
    fn uninstall(&self) -> anyhow::Result<()> {
        Ok(())
    }
    fn current(&self) -> Option<WindowReport> {
        None
    }
    fn spawn_watch(self: std::sync::Arc<Self>, _tx: SyncSender<Option<WindowReport>>) -> anyhow::Result<()> {
        Ok(())
    }
}
