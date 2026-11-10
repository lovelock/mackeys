//! Per-device reader: grab-with-idle-wait (faithful port of xwaykeyz
//! `devices.py`: `_wait_for_device_idle` / `_drain_buffered_events` /
//! grab retry loop), then blocking event pump → channel.
//!
//! Why the idle wait matters (Toshy's comment, ported): a grab must not span
//! a key release the desktop is waiting for — the desktop saw the press but
//! only we get the release, and libinput treats that key as held forever.
//! So: wait for the device to be quiet (no held keys, no buffered key events,
//! 150 ms silence), grab immediately, verify kernel state; if keys are held
//! across the grab, ungrab and retry within a 5 s budget. (We deliberately
//! do NOT replay held keys as presses — that was a previous invention and
//! the opposite of what the desktop needs.)

use super::devices::DeviceInfo;
use crate::types::Act;
use anyhow::{Context, Result};
use evdev::{Device, EventType};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub enum LoopMsg {
    Act(usize, Act),
    /// device disappeared — main loop drops its state
    Eof(usize),
    Focus(Option<crate::adapters::WindowReport>), // full report; bool computed at consumer
    /// kernel uevent says devices changed → rescan_tick
    Rescan,
}

const MAX_IDLE_WAIT: Duration = Duration::from_secs(5);
const QUIET_PERIOD: Duration = Duration::from_millis(150);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const GRAB_TRIES: u32 = 9;
const GRAB_DELAY_BASE: Duration = Duration::from_millis(200);

/// Spawn one blocking reader thread for a managed device.
pub fn spawn_reader(
    info: DeviceInfo,
    tag: usize,
    tx: Sender<LoopMsg>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let mut dev =
        Device::open(&info.path).with_context(|| format!("open {}", info.path.display()))?;
    // python-evdev opens input devices O_RDWR|O_NONBLOCK; mirror that so the
    // idle-wait / drain helpers can poll it without blocking forever.
    set_nonblocking(&dev);

    grab_with_idle_wait(&mut dev, &info, &stop)?;

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
                        std::thread::sleep(POLL_INTERVAL);
                    }
                    Err(e) => {
                        tracing::warn!(tag, error = %e, "reader stream error — device lost (hotplug/suspend)");
                        break;
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

fn set_nonblocking(dev: &Device) {
    use std::os::fd::AsRawFd;
    unsafe {
        let fd = dev.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

fn held_keys(dev: &Device) -> Vec<u16> {
    match dev.get_key_state() {
        Ok(state) => state.iter().map(|k| k.0).collect(),
        Err(_) => vec![],
    }
}

/// Drain anything already buffered on our fd (Python: `_drain_buffered_events`).
fn drain_buffered(dev: &mut Device) {
    if let Ok(events) = dev.fetch_events() {
        for _ in events { /* discard */ }
    }
}

/// Wait until the device has no held keys and no buffered KEY events for
/// `QUIET_PERIOD`, up to `budget`. Returns true when idle.
fn wait_for_idle(dev: &mut Device, budget: Duration) -> bool {
    let start = Instant::now();
    let mut last_activity = Instant::now();
    while start.elapsed() < budget {
        let held = held_keys(dev);
        if !held.is_empty() {
            last_activity = Instant::now();
        }
        // any buffered key event also resets the idle timer
        if let Ok(events) = dev.fetch_events() {
            for ev in events {
                if ev.event_type() == EventType::KEY {
                    last_activity = Instant::now();
                }
            }
        }
        if last_activity.elapsed() >= QUIET_PERIOD {
            return true;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    false
}

/// Faithful port of xwaykeyz `grab()`: idle wait, grab immediately, verify
/// exact kernel state, undo+retry (shared 5 s budget) if a key is held
/// across the grab, exponential retry on grab errors.
fn grab_with_idle_wait(dev: &mut Device, info: &DeviceInfo, stop: &Arc<AtomicBool>) -> Result<()> {
    let deadline = Instant::now() + MAX_IDLE_WAIT;
    let _ = wait_for_idle(dev, MAX_IDLE_WAIT);
    if stop.load(Ordering::Relaxed) {
        anyhow::bail!("stop requested");
    }

    let mut delay = GRAB_DELAY_BASE;
    for attempt in 1..=GRAB_TRIES {
        match dev.grab() {
            Ok(()) => {
                drain_buffered(dev);
                let held = held_keys(dev);
                if held.is_empty() {
                    tracing::info!(name = %info.name, path = %info.path.display(), "grabbed");
                    return Ok(());
                }
                // keys held across the grab: undo, wait for idle again while
                // the 5 s budget remains (Toshy: don't hold the release the
                // desktop is waiting for)
                tracing::debug!(name = %info.name, ?held, "keys held across grab — ungrab and wait");
                let _ = dev.ungrab();
                if Instant::now() >= deadline {
                    // budget exhausted: grab anyway (Toshy does the same and
                    // relies on stuck-key repair for the rare straggler)
                    dev.grab().context("re-grab past idle budget")?;
                    drain_buffered(dev);
                    tracing::info!(name = %info.name, "grabbed (past idle budget)");
                    return Ok(());
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                let _ = wait_for_idle(dev, remaining);
            }
            Err(e) => {
                // Toshy: 9 tries, exponential backoff (EBUSY / perms settling)
                tracing::warn!(
                    name = %info.name,
                    attempt,
                    error = %e,
                    "grab failed — retrying"
                );
                std::thread::sleep(delay);
                delay = std::cmp::min(delay * 2, Duration::from_secs(60));
            }
        }
    }
    anyhow::bail!("grab retries exhausted for {}", info.name)
}
