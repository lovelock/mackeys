//! Device enumeration + classification (spec §8: only letter-capable nodes).

use crate::adapters;
use evdev::Device;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub name: String,
    pub path: PathBuf,
    pub letter: bool,
    pub managed: bool,
}

pub const SELF_VIRTUAL_NAME: &str = "mackeys virtual keyboard";

/// Pure decision function (unit-tested): which devices mackeys takes over.
pub fn classify(
    name: Option<&str>,
    has_letter_keys: bool,
    excludes: &[String],
) -> bool {
    if !has_letter_keys {
        return false; // mice / consumer-control / system-control nodes stay native
    }
    let Some(n) = name else { return false };
    if n == SELF_VIRTUAL_NAME {
        return false; // never grab our own output
    }
    if n.to_lowercase().contains("virtual") {
        // lab isolation hook: MACKEYS_TEST_GRAB lets a test rig grab its own
        // synthetic keyboard (name match) so the pipeline can be exercised
        // without touching the physical devices
        if let Ok(pat) = std::env::var("MACKEYS_TEST_GRAB") {
            if !pat.is_empty() && n.contains(&pat) {
                return true;
            }
        }
        return false; // other remappers' virtual devices (XWayKeyz etc.) — no recursion
    }
    if excludes.iter().any(|x| n.contains(x.as_str())) {
        return false;
    }
    true
}

/// Enumerate /dev/input/event* and classify.
/// (devices, saw_permission_race) — true when some node was not yet
/// accessible, which happens for a few ms after udev creates it.
pub fn scan(excludes: &[String]) -> anyhow::Result<Vec<DeviceInfo>> {
    let (devs, _race) = scan_with_race(excludes)?;
    Ok(devs)
}

pub fn scan_with_race(excludes: &[String]) -> anyhow::Result<(Vec<DeviceInfo>, bool)> {
    let mut saw_race = false;
    let mut infos = Vec::new();
    for entry in std::fs::read_dir("/dev/input")? {
        let entry = entry?;
        let path = entry.path();
        let fname = path.to_string_lossy().to_string();
        if !fname.contains("/event") || fname.contains("by-path") {
            continue;
        }
        let dev = match Device::open(&path) {
            Ok(d) => d,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    saw_race = true; // udev still applying group/uaccess
                }
                tracing::debug!(path = %fname, error = %e, "cannot open device");
                continue;
            }
        };
        let name = dev.name().unwrap_or("<unnamed>").to_string();
        let letter = has_letter_keys(&dev);
        infos.push(DeviceInfo {
            name,
            path: path.clone(),
            letter,
            managed: classify(dev.name(), letter, excludes),
        });
    }
    infos.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((infos, saw_race))
}

fn has_letter_keys(dev: &Device) -> bool {
    // KEY_K = 37, KEY_C = 46 — presence of letter codes marks a keyboard node.
    dev.supported_keys()
        .map(|keys| keys.contains(evdev::KeyCode(37)) && keys.contains(evdev::KeyCode(46)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_selects_real_keyboards_only() {
        assert!(classify(Some("BY Tech Gaming Keyboard"), true, &[]));
        assert!(classify(Some("YICHIP Wireless Device"), true, &[]));
        assert!(!classify(Some("BY Tech Gaming Keyboard Consumer Control"), false, &[]));
        assert!(!classify(Some("Logitech USB Receiver Mouse"), false, &[]));
        assert!(!classify(Some("mackeys virtual keyboard"), true, &[]), "self device");
        assert!(
            !classify(Some("XWayKeyz (virtual) Keyboard"), true, &[]),
            "other remappers' virtual devices are never grabbed"
        );
        assert!(
            !classify(Some("BY Tech Gaming Keyboard"), true, &["Gaming".to_string()]),
            "exclude list"
        );
        assert!(!classify(None, true, &[]), "unnamed device refused");
    }

    #[test]
    fn is_terminal_matches_case_insensitively() {
        use crate::adapters::{is_terminal, WindowReport};
        let classes = vec!["^konsole".to_string(), "ghostty".to_string()];
        let konsole = WindowReport { caption: "shell".into(), resource_class: "org.kde.Konsole".into(), resource_name: "zsh".into() };
        assert!(is_terminal(&konsole, &classes));
        let konsole2 = WindowReport { caption: "shell".into(), resource_class: "".into(), resource_name: "Konsole".into() };
        assert!(is_terminal(&konsole2, &classes));
        let browser = WindowReport { caption: "GitHub".into(), resource_class: "chromium".into(), resource_name: "browser".into() };
        assert!(!is_terminal(&browser, &classes));
        let empty = WindowReport { caption: "".into(), resource_class: "".into(), resource_name: "".into() };
        assert!(!is_terminal(&empty, &classes));
    }

    #[test]
    fn window_report_is_the_interface_sharing_type() {
        // compile-level pin: adapters::WindowReport exists (FocusSource contract)
        let r = adapters::WindowReport {
            caption: "x".into(),
            resource_class: "y".into(),
            resource_name: "z".into(),
        };
        assert_eq!(r.caption, "x");
    }
}
