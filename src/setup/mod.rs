//! setup / doctor / service control. Tasks 1+8 (core plan).

pub fn config_path() -> anyhow::Result<std::path::PathBuf> {
    let dir = std::env::var("XDG_CONFIG_HOME")
        .map(|h| std::path::PathBuf::from(h).join("mackeys"))
        .unwrap_or_else(|_| {
            dirs_home().map(|h| h.join(".config/mackeys")).unwrap_or_else(|| std::path::PathBuf::from("."))
        });
    Ok(dir.join("config.toml"))
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(std::path::PathBuf::from)
}

pub fn setup() -> anyhow::Result<()> {
    anyhow::bail!("setup lands with Task 8")
}

pub fn doctor() -> anyhow::Result<()> {
    anyhow::bail!("doctor lands with Task 8")
}

pub fn service_ctl(_cmd: &str) -> anyhow::Result<()> {
    anyhow::bail!("service control lands with Task 8")
}

pub fn devices() -> anyhow::Result<()> {
    anyhow::bail!("devices lands with Task 8 (real device list lives in daemon/devices.rs)")
}

pub fn uninstall() -> anyhow::Result<()> {
    anyhow::bail!("uninstall lands with Task 8")
}
