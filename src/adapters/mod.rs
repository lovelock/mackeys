//! DE focus adapters — replaceable component (spec §4).
//! daemon core depends ONLY on the FocusSource trait; kwin-specific details
//! live in kwin.rs and never escape this module.

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
        tx: std::sync::mpsc::SyncSender<Option<WindowReport>>,
    ) -> anyhow::Result<()>;
}

/// Terminal-class matching: case-insensitive over resourceClass first, then
/// resourceName, then caption (spec §8: teachable via config list).
pub fn is_terminal(report: &WindowReport, classes: &[String]) -> bool {
    for pat in classes {
        let p = pat.trim();
        let (neg, p) = match p.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, p),
        };
        let rx = match p.strip_prefix('^') {
            Some(prefix) => prefix.to_lowercase(),
            None => p.to_lowercase(),
        };
        let hay = if !report.resource_class.is_empty() {
            report.resource_class.to_lowercase()
        } else if !report.resource_name.is_empty() {
            report.resource_name.to_lowercase()
        } else {
            report.caption.to_lowercase()
        };
        let m = hay.contains(&rx);
        if m {
            return !neg;
        }
    }
    false
}

pub mod kwin;
pub mod none;

pub fn best_for(config_adapter: &str) -> anyhow::Result<Box<dyn FocusSource>> {
    let env_desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    let chosen = match config_adapter {
        "auto" => {
            if env_desktop.contains("kde") || std::env::var("KWIN_RUNNING").is_ok() {
                "kwin"
            } else {
                "none"
            }
        }
        other => other,
    };
    match chosen {
        "kwin" => Ok(Box::new(kwin::KwinAdapter::new())),
        "none" => Ok(Box::new(none::NoneAdapter)),
        "gnome" => Err(anyhow::anyhow!("gnome adapter not implemented yet (spec §9 backlog)")),
        "windows" => Err(anyhow::anyhow!("windows adapter not implemented yet (spec §9 backlog)")),
        other => Err(anyhow::anyhow!("adapter '{}' invalid", other)),
    }
}
