//! Task 8: setup idempotency — hermetic integration test (needs CARGO_BIN_EXE).

use std::process::Command;

#[test]
fn setup_idempotent() {
    let dir = std::env::temp_dir().join(format!("mackeys-integ-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let xdg = dir.join("xdg");
    let home = dir.join("home");
    std::fs::create_dir_all(&xdg).unwrap();
    std::fs::create_dir_all(&home).unwrap();

    let envs = [
        ("XDG_CONFIG_HOME", xdg.to_string_lossy().to_string()),
        ("HOME", home.to_string_lossy().to_string()),
        ("XDG_CURRENT_DESKTOP", "mackeys-test-env".to_string()),
    ];

    let bin = env!("CARGO_BIN_EXE_mackeys");

    // first run — files created
    let out1 = Command::new(bin)
        .args(["setup"])
        .envs(envs.clone())
        .output()
        .expect("run setup #1");
    assert!(out1.status.success(), "setup #1 rc: {}", out1.status);
    let cfg = xdg.join("mackeys/config.toml");
    let unit = xdg.join("systemd/user/mackeys.service");
    assert!(cfg.exists(), "config written");
    assert!(unit.exists(), "unit written");
    let cfg_1 = std::fs::read(&cfg).unwrap();
    let unit_1 = std::fs::read(&unit).unwrap();
    assert!(
        String::from_utf8_lossy(&out1.stdout).contains("wrote config template"),
        "first run writes config"
    );

    // second run — all "already", byte-identical
    let out2 = Command::new(bin)
        .args(["setup"])
        .envs(envs.clone())
        .output()
        .expect("run setup #2");
    assert!(out2.status.success(), "setup #2 rc");
    let stdout2 = String::from_utf8_lossy(&out2.stdout);
    assert!(stdout2.contains("already: config exists"), "cfg already: {stdout2}");
    assert!(stdout2.contains("already: unit file exists"), "unit already");
    assert_eq!(cfg_1, std::fs::read(&cfg).unwrap(), "config byte-identical");
    assert_eq!(unit_1, std::fs::read(&unit).unwrap(), "unit byte-identical");

    // third run with --dry-run — still nothing written
    let out3 = Command::new(bin)
        .args(["setup", "--dry-run"])
        .envs(envs)
        .output()
        .expect("run setup --dry-run");
    assert!(out3.status.success());
    assert!(String::from_utf8_lossy(&out3.stdout).contains("dry-run"));
    assert_eq!(cfg_1, std::fs::read(&cfg).unwrap(), "dry-run untouched");
    assert_eq!(unit_1, std::fs::read(&unit).unwrap(), "dry-run untouched");

    let _ = std::fs::remove_dir_all(&dir);
}
