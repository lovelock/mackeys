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

/// kwinrc [Plugins] persistence key = metadata Id literal + "Enabled".
pub fn kwinrc_enabled_key(script_id: &str) -> String {
    format!("{script_id}Enabled")
}

/// Generic retry-with-delay (testable pure plumbing for adapter installs).
pub fn poll_until(mut tries: u32, delay: std::time::Duration, mut f: impl FnMut() -> bool) -> bool {
    loop {
        if f() {
            return true;
        }
        if tries <= 1 {
            return false;
        }
        tries -= 1;
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }
}

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

#[cfg(test)]
mod adapters_tests {
    use super::poll_until;
    use std::cell::Cell;

    #[test]
    fn poll_until_returns_true_when_pred_succeeds() {
        let count = Cell::new(0u32);
        let ok = poll_until(5, std::time::Duration::ZERO, || {
            count.set(count.get() + 1);
            count.get() >= 3
        });
        assert!(ok, "pred true at 3rd call must succeed within 5 tries");
    }

    #[test]
    fn poll_until_returns_false_when_exhausted() {
        let count = Cell::new(0u32);
        let ok = poll_until(3, std::time::Duration::ZERO, || {
            count.set(count.get() + 1);
            false
        });
        assert!(!ok);
        assert_eq!(count.get(), 3, "tried exactly 3 times");
    }
}

#[cfg(test)]
mod kwin_key_tests {
    use super::kwinrc_enabled_key;
    // review C3 regression pin: kwinrc plugins key must be metadata-Id literal
    // + "Enabled" (mackeys-focusEnabled), NEVER camelCase.
    #[test]
    fn enabled_key_uses_id_literal() {
        assert_eq!(kwinrc_enabled_key("mackeys-focus"), "mackeys-focusEnabled");
    }
}
