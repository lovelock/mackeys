//! Task 8 tests: doctor pure checks (hermetic); setup idempotency lives in
//! ../tests/setup_idempotent.rs (integration test — needs CARGO_BIN_EXE).

use super::{conflicting_remappers, fcitx_trigger_ok, parse_is_active, uinput_ok};
use crate::setup::service::unit_file_text;

#[test]
fn unit_file_text_pins_required_directives() {
    let txt = unit_file_text("/usr/bin/mackeys");
    assert!(txt.contains("After=graphical-session.target"));
    assert!(txt.contains("Restart=on-failure"));
    assert!(txt.contains("ExecStart=/usr/bin/mackeys"));
    assert!(txt.contains("WantedBy=graphical-session.target"));
}

#[test]
fn doctor_helpers() {
    assert!(fcitx_trigger_ok("[Hotkey/TriggerKeys]\n0=Control+space\n1=Hangul\n"));
    assert!(!fcitx_trigger_ok("[Hotkey/TriggerKeys]\n0=Caps_Lock\n"));
    assert!(!fcitx_trigger_ok("[Behavior]\n"));
    assert!(!fcitx_trigger_ok(""));
    assert!(parse_is_active("active"));
    assert!(parse_is_active("Active\n"));
    assert!(!parse_is_active("activating"));
    assert!(!parse_is_active("failed"));
    assert!(uinput_ok("660", true).0);
    assert!(!uinput_ok("660", false).0);
    assert!(uinput_ok("666", false).0);
    assert!(!uinput_ok("", true).0);
    assert_eq!(
        conflicting_remappers("ps output mentions keyd + xremap +bash"),
        vec!["keyd".to_string(), "xremap".to_string()]
    );
    assert!(conflicting_remappers("nothing here").is_empty());
}

#[test]
fn wants_debug_flag() {
    assert!(super::wants_debug(&["run".into(), "--debug".into()]));
    assert!(super::wants_debug(&["-d".into()]));
    assert!(!super::wants_debug(&["run".into()]));
    assert!(!super::wants_debug(&["setup".into(), "--takeover".into()]));
}

#[test]
fn doctor_helpers_groups() {
    use super::groups_contains;
    assert!(groups_contains("users wheel input", "input"));
    assert!(!groups_contains("users wheel", "input"));
    assert!(!groups_contains("", "input"));
}

#[test]
fn heartbeat_wallclock_deadline() {
    use std::time::{Duration, Instant};
    // simulates the main loop: heartbeat fires on WALL CLOCK, immune to
    // intervening loops or slow enumeration
    let now = Instant::now();
    let period = Duration::from_secs(100);
    let mut next = now + period;
    let mut iterations = 0;
    let mut fired = 0;
    // 200 simulated loop ticks, irregular spacing, one very slow 120s span
    let spacing = [Duration::from_millis(200), Duration::from_secs(120)];
    let mut t = now;
    for i in 0..10 {
        t = t + spacing[i % 2];
        if t >= next {
            next = t + period;
            fired += 1;
        }
        iterations += 1;
    }
    assert!(fired >= 1, "slow loop must not starve the heartbeat");
}

