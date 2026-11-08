//! setup / doctor / service control. Tasks 1+8 (core plan).

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
