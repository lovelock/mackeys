//! Doctor: environment checks, each a pure function over inputs; the CLI
//! entry wires live probes to them and prints PASS/FAIL lines.

use anyhow::Result;
use super::{conflicting_remappers, fcitx_trigger_ok, parse_is_active, uinput_ok};

pub fn conflicting_remappers_ok(procs: &str) -> (bool, String) {
    let found = conflicting_remappers(procs);
    if found.is_empty() {
        (true, String::new())
    } else {
        (false, format!("running: {found:?} — stop or uninstall them first"))
    }
}

pub fn fcitx_ok(cfg_text: &str) -> (bool, String) {
    if cfg_text.is_empty() {
        return (true, "fcitx5 config absent — ibus/GNOME default works".into());
    }
    if fcitx_trigger_ok(cfg_text) {
        (true, "TriggerKeys[0]=Control+space".into())
    } else {
        (false, "TriggerKeys[0] != Control+space — fix in fcitx5 settings".into())
    }
}

pub fn unit_active(out: &str) -> (bool, String) {
    if parse_is_active(out) {
        (true, "running".into())
    } else {
        (false, format!("state '{}' — systemctl --user enable --now mackeys", out.trim()))
    }
}

pub fn uinput(status_line: &str, in_group_input: bool) -> (bool, String) {
    uinput_ok(status_line, in_group_input)
}

// ---- CLI wiring ----

pub fn run() -> Result<String> {
    let mut report = String::new();
    let mut line = |ok: bool, name: &str, detail: String| {
        let mark = if ok { "PASS" } else { "FAIL" };
        report.push_str(&format!("[{mark}] {name}: {detail}\n"));
    };

    // 1. uinput node
    let stat = std::process::Command::new("stat")
        .args(["-c", "%a", "/dev/uinput"])
        .output()
        .ok();
    let stat_line = match &stat {
        Some(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    };
    let (ok, detail) = uinput(&stat_line, super::in_group("input"));
    line(ok, "uinput", detail);

    // 2. input group
    let in_input = super::in_group("input");
    line(in_input, "input group", if in_input { "user in input".into() } else { "add with: usermod -aG input $USER".into() });

    // 3. unit active
    let out = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "mackeys"])
        .output()
        .ok();
    let is_active = out
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_else(|| "unknown".into());
    let (ok, detail) = unit_active(&is_active);
    line(ok, "unit", detail);

    // 4. conflicting remappers
    let psout = std::process::Command::new("sh")
        .arg("-c")
        .arg("ps -eo args= 2>/dev/null || true")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let (ok, detail) = conflicting_remappers_ok(&psout);
    line(ok, "no conflicting remappers", detail);

    // 5. fcitx5 trigger
    let fx = std::fs::read_to_string(super::user_config_dir().join("fcitx5/config"))
        .unwrap_or_default();
    let (ok, detail) = fcitx_ok(&fx);
    line(ok, "fcitx5 trigger", detail);

    // 6. KRunner Alt+Space defaults visible
    let krunner = std::fs::read_to_string("/usr/share/kglobalaccel/org.kde.krunner.desktop")
        .unwrap_or_default();
    line(
        !krunner.is_empty() && krunner.contains("Alt+Space"),
        "KRunner Alt+Space",
        if krunner.contains("Alt+Space") {
            "desktop defaults carry Alt+Space (Cmd+Space → Alt+Space wired)".into()
        } else {
            "not found on this system — check KDE shortcut settings".into()
        },
    );

    // 7. adapter install state
    if let Ok(adapter) = crate::adapters::best_for("auto") {
        match adapter.id() {
            "none" => line(true, "adapter", "none (no context reports; terminals need manual config override)".into()),
            "kwin" => {
                let loaded = kwin_script_loaded();
                line(
                    loaded.as_ref().map(|v| *v).unwrap_or(false),
                    "kwin adapter",
                    match loaded {
                        Ok(true) => "script loaded".into(),
                        Ok(false) => "script NOT loaded — run `mackeys setup`".into(),
                        Err(e) => format!("check failed: {e}"),
                    },
                );
            }
            other => line(false, "adapter", format!("unsupported id {other}")),
        }
    }

    println!("{report}");
    Ok(report)
}

fn kwin_script_loaded() -> Result<bool> {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(
            "busctl call --user org.kde.KWin /Scripting org.kde.kwin.Scripting isScriptLoaded s \"mackeys-focus\"",
        )
        .output()?;
    let txt = String::from_utf8_lossy(&out.stdout).to_lowercase();
    let _ = super::SELF_UNITS.len(); // keep use
    Ok(out.status.success() && txt.contains("true"))
}
