//! setup / doctor / service control — CLI surface (Task 8).

pub mod doctor;
pub mod service;

use anyhow::Result;

pub const SELF_UNITS: &[&str] = &[
    "toshy-config",
    "toshy-kwin-dbus",
    "toshy-session-monitor",
    "toshy-cosmic-dbus",
    "toshy-wlroots-dbus",
    "toshy-var",
    "app-Toshy_Tray@autostart",
];

/// Canonical config path (respects XDG_CONFIG_HOME).
pub fn config_path() -> Result<std::path::PathBuf> {
    Ok(user_config_dir().join("mackeys/config.toml"))
}


pub const DEFAULT_CONFIG_TOML: &str = r#"# mackeys 配置 — 默认值即作者偏好，只写需要覆盖的项
[general]
# kbtype_default = "windows"            # windows | apple
# adapter = "auto"                      # auto | kwin | gnome | none | windows
# swap_caps_lctrl = true
# disable_builtins = ["cmd_tab"]        # 按内置 id 关闭规则
# exclude_devices = ["Gaming Keyboard"] # 按名排除设备

[targets]
terminal_classes = ["^konsole", "ghostty", "alacritty", "kitty", "wezterm", "foot"]

[launcher]
# cmd_space = "alt+space"               # Cmd+Space 去哪儿；"none" = 吞键

# [[rules]]                              # 用户规则（first-match-wins，排最前）
# input  = "cmd+w"
# output = "ignore"
# context = "gui"                        # gui | term | any
"#;

// ---------------- doctor pure helpers (used by doctor.rs) ----------------

/// other remappers currently grabbing devices
pub fn conflicting_remappers(process_list: &str) -> Vec<String> {
    ["keyd", "xremap", "xwaykeyz"]
        .iter()
        .filter(|n| process_list.contains(*n))
        .map(|n| n.to_string())
        .collect()
}

/// fcitx5 trigger: does [Hotkey/TriggerKeys] start with Control+space?
pub fn fcitx_trigger_ok(cfg_text: &str) -> bool {
    let mut in_sec = false;
    for line in cfg_text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_sec = l.starts_with("[Hotkey/TriggerKeys]");
            continue;
        }
        if in_sec && l.starts_with("0=") {
            return l.eq_ignore_ascii_case("0=Control+space");
        }
    }
    false
}

/// systemctl is-active output → running?
pub fn parse_is_active(out: &str) -> bool {
    out.trim().eq_ignore_ascii_case("active")
}

/// uinput permission from stat mode output, e.g. "660" + group knowledge
pub fn uinput_ok(mode: &str, user_in_input_group: bool) -> (bool, String) {
    let m = mode.trim();
    if m.is_empty() {
        return (false, "/dev/uinput missing".into());
    }
    if m == "660" && user_in_input_group {
        (true, format!("mode {m} + input group"))
    } else if m == "666" || m == "606" {
        (true, format!("mode {m} (world-writable)"))
    } else {
        (
            false,
            format!("mode {m}: needs udev rule or input-group membership"),
        )
    }
}

fn user_config_dir() -> std::path::PathBuf {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return std::path::PathBuf::from(x);
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home).join(".config")
}

pub fn in_group(group: &str) -> bool {
    // review I4: /etc/group membership strings are not evidence — ask the
    // kernel what groups the CURRENT user actually has (incl. primary group)
    let out = std::process::Command::new("id")
        .args(["-Gn"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    groups_contains(&out, group)
}

/// pure: does `groups` (id -Gn style whitespace list) contain group?
pub fn groups_contains(groups: &str, group: &str) -> bool {
    groups.split_whitespace().any(|g| g == group)
}

/// main-level flag: enable per-key debug tracing for the daemon.
pub fn wants_debug(args: &[String]) -> bool {
    args.iter().any(|a| a == "--debug" || a == "-d")
}

// ---------------- CLI ----------------

pub fn setup() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    setup_with(&args[1..])
}

pub fn setup_with(args: &[String]) -> Result<()> {
    let dry = args.iter().any(|a| a == "--dry-run");
    let takeover = args.iter().any(|a| a == "--takeover");
    do_setup(dry, takeover)
}

fn do_setup(dry: bool, takeover: bool) -> Result<()> {
    println!("mackeys setup{}", if dry { " (dry-run)" } else { "" });

    // 1. config template (idempotent)
    let cfg_path = user_config_dir().join("mackeys/config.toml");
    if cfg_path.exists() {
        println!("already: config exists at {} (kept)", cfg_path.display());
    } else if dry {
        println!("would write config template to {}", cfg_path.display());
    } else {
        if let Some(parent) = cfg_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&cfg_path, DEFAULT_CONFIG_TOML)?;
        println!("wrote config template: {}", cfg_path.display());
    }

    // 2. unit file (idempotent)
    let exe = std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "/usr/local/bin/mackeys".into());
    let xdg = user_config_dir();
    let (written, unit_path) = service::write_unit(&xdg, &exe, dry)?;
    if written && !dry {
        println!("wrote unit file: {}", unit_path.display());
    } else if written && dry {
        println!("would write unit file: {}", unit_path.display());
    } else {
        println!("already: unit file exists at {} (kept)", unit_path.display());
    }

    // 3. adapter install (auto detection; in hermetic env → none adapter)
    if dry {
        println!("would install focus adapter");
    } else if let Ok(adapter) = crate::adapters::best_for("auto") {
        match adapter.install() {
            Ok(()) => println!("focus adapter '{}' installed", adapter.id()),
            Err(e) => tracing::warn!(error = %e, "adapter install failed (skipped)"),
        }
    }

    // 4. systemctl daemon-reload
    if dry {
        println!("would run: systemctl --user daemon-reload");
    } else {
        let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    }

    // 5. toshy takeover
    if takeover && !dry {
        takeover_toshy();
    } else if takeover && dry {
        println!("would disable toshy units");
    }

    if dry {
        println!("(dry-run: nothing above was written)");
        return Ok(());
    }

    println!("next: systemctl --user enable --now mackeys");
    println!("—— doctor ——");
    doctor::run()?;
    Ok(())
}

fn takeover_toshy() {
    for unit in SELF_UNITS {
        let res = std::process::Command::new("systemctl")
            .args(["--user", "disable", "--now", unit])
            .output();
        match res {
            Ok(o) if o.status.success() => println!("disabled: {unit}"),
            _ => println!("skip: {unit} (not found or refused)"),
        }
    }
}

pub fn doctor() -> Result<()> {
    doctor::run()?;
    Ok(())
}

pub fn service_ctl(cmd: &str) -> Result<()> {
    let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    let st = std::process::Command::new("systemctl").args(["--user", cmd, "mackeys"]).status()?;
    if !st.success() {
        anyhow::bail!("systemctl --user {cmd} mackeys failed");
    }
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", "mackeys"])
        .status();
    if let Ok(adapter) = crate::adapters::best_for("auto") {
        let _ = adapter.uninstall();
    }
    let unit = user_config_dir().join("systemd/user/mackeys.service");
    if unit.exists() {
        std::fs::remove_file(&unit)?;
        println!("removed unit: {}", unit.display());
    }
    println!(
        "mackeys uninstalled (config left at {})",
        user_config_dir().join("mackeys/config.toml").display()
    );
    Ok(())
}

pub fn devices() -> Result<()> {
    let infos = crate::daemon::devices::scan(&[])?;
    println!("device inventory:");
    for i in &infos {
        println!("  {:10} {}", if i.managed { "[managed]" } else { "[native ]" }, i.name);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
