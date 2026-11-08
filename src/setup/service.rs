//! systemd user unit generation (idempotent writer, Task 8).

use anyhow::Result;

pub const UNIT_NAME: &str = "mackeys.service";

pub fn unit_file_text(exe: &str) -> String {
    format!(
        r#"[Unit]
Description=mackeys — macOS keymap daemon (evdev→uinput)
After=graphical-session.target

[Service]
Type=simple
ExecStart={exe}
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
"#
    )
}

/// Returns (written?, unit path). `dry` performs no writes.
pub fn write_unit(
    xdg_config: &std::path::Path,
    exe: &str,
    dry: bool,
) -> Result<(bool, std::path::PathBuf)> {
    let dir = xdg_config.join("systemd/user");
    let path = dir.join(UNIT_NAME);
    let txt = unit_file_text(exe);
    if path.exists() {
        Ok((false, path))
    } else {
        if !dry {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(&path, txt)?;
        }
        Ok((true, path))
    }
}
