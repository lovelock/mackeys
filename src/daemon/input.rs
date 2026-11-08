//! Per-device blocking reader loop: grab, keystate sync, events → channel.

use super::devices::DeviceInfo;
use crate::types::Act;
use anyhow::{Context, Result};
use evdev::{Device, EventType};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

pub enum LoopMsg {
    Act(usize, Act),
    /// device disappeared — main loop drops its state
    Eof(usize),
    Focus(Option<crate::adapters::WindowReport>), // full report; bool computed at consumer
}

pub struct ReaderHandle {
    pub dev: Device,
}

/// Spawn one blocking reader thread for a managed device.
/// On exit (stop flag or device EOF) it ungrabs and closes.
pub fn spawn_reader(
    info: DeviceInfo,
    tag: usize,
    tx: Sender<LoopMsg>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let mut dev = Device::open(&info.path).with_context(|| format!("open {}", info.path.display()))?;
    dev.grab()
        .with_context(|| format!("grab {}", info.name))?;

    // keystate sync at grab time (spec §8): modifiers already held when we
    // take over must be present in the engine state.
    let held_now = down_ids(&dev);
    if !held_now.is_empty() {
        tracing::debug!(name=%info.name, ?held_now, "initial held keys");
        for code in held_now {
            let _ = tx.send(LoopMsg::Act(tag, Act { code, value: 1 }));
        }
    }

    std::thread::Builder::new()
        .name(format!("mackeys-reader-{}", info.name))
        .spawn(move || {
            'outer: loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                match dev.fetch_events() {
                    Ok(events) => {
                        for ev in events {
                            if ev.event_type() != EventType::KEY {
                                continue; // evdev crate resyncs SYN internally
                            }
                            let act = Act { code: ev.code(), value: ev.value() };
                            if tx.send(LoopMsg::Act(tag, act)).is_err() {
                                tracing::warn!(tag, "main loop gone — reader exiting");
                                break 'outer;
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    Err(e) => {
                        tracing::warn!(tag, error = %e, "reader stream error — device lost (hotplug/suspend)");                        break;
                    }
                }
            }
            match dev.ungrab() {
                Ok(()) => tracing::debug!(tag, "ungrab ok"),
                Err(e) => tracing::warn!(tag, error = %e, "ungrab failed (device already gone — ENODEV expected on hardware unplug)"),
            }
            let _ = tx.send(LoopMsg::Eof(tag));
        })?;
    Ok(())
}

fn down_ids(dev: &Device) -> Vec<u16> {
    let mut out = Vec::new();
    if let Ok(state) = dev.get_key_state() {
        for key in 0..=249u16 {
            if state.contains(evdev::KeyCode(key)) {
                out.push(key);
            }
        }
    }
    out
}
