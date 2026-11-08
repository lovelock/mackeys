//! uinput sink — clean-room implementation over libc.
//!
//! Why not evdev's `VirtualDevice`: its `write()` can BLOCK forever when the
//! kernel's uinput ring buffer fills up and the compositor (KWin) stops
//! consuming `/dev/input/event25` — incident 2026-11-08: whole keyboard mute
//! for minutes while the daemon stayed "active". We open `/dev/uinput`
//! ourselves with `O_NONBLOCK`, write from a dedicated thread, and on a
//! sustained stall (EAGAIN) we DROP pending events instead of blocking; on
//! recovery we re-release every modifier/key the OS may still believe held.

use anyhow::{Context, Result};
use crate::types::Out;
use std::collections::BTreeSet;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

// ---------------------------------------------------------------------------
// ioctl plumbing (linux/uinput.h)
// ---------------------------------------------------------------------------

/// _IOW('U', nr, int-style size with direction bits folded correctly)
const fn ui_iow_set(nr: u16, payload_size: u64) -> u64 {
    // _IOC_WRITE = 1
    (1 << 30) | ((payload_size & 0x3fff) << 16) | ((b'U' as u64 & 0xff) << 8) | (nr as u64 & 0xff)
}
/// _IO('U', nr) — no direction, no payload size
const fn ui_io_dev(nr: u16) -> u64 {
    (b'U' as u64 & 0xff) << 8 | (nr as u64 & 0xff)
}
/// `_IOW('U', nr, int)` — payload size is `c_int` (4).
const fn ui_set_evbit(nr: u16, payload_size: u64) -> u64 {
    // _IOW: direction=1 (_IOC_WRITE)
    (1 << 30) | ((payload_size & 0x3fff) << 16) | ((b'U' as u64 & 0xff) << 8) | (nr as u64 & 0xff)
}

const UI_SET_EVBIT: u64 = ui_iow_set(100, 4);
const UI_SET_KEYBIT: u64 = ui_iow_set(101, 4);
const UI_DEV_CREATE: u64 = ui_io_dev(1);
const UI_DEV_DESTROY: u64 = ui_io_dev(2);

const EV_KEY: u16 = 0x01;
const EV_SYN: u16 = 0x00;
const SYN_REPORT: u16 = 0;

const UINPUT_PATH: &str = "/dev/uinput";
const VIRT_NAME: &str = "mackeys virtual keyboard";

// ---------------------------------------------------------------------------
// kernel structures (done with exact-offset byte packing, layout-stable)
// ---------------------------------------------------------------------------

#[repr(C)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

// uinput_user_dev is name[80] + input_id(4×u16) + ff_effects_max(u32) + the
// abs/rel/led/rep/snd/sw coordinate arrays (zeroed = no axes configured).
// We mirror byte-for-byte; sizes below match x86_64 + generic kernel.
// exact sizeof(struct uinput_user_dev): name[80] + input_id(8) +
// ff_effects_max(4) + absmax/absmin/absfuzz/absflat (4 arrays × 64 × 4)
const UINPUT_USER_DEV_BYTES: usize = 80 + 8 + 4 + (4 * 4 * 64);

// ---------------------------------------------------------------------------
// StallTracker — what to do when writes fail and when they recover
// ---------------------------------------------------------------------------

/// Tracks which output keys the OS currently believes are pressed
/// (successful writes only), so that a stalled release can be flushed on
/// recovery. Failures of a PRESS are simply not remembered (the OS never
/// saw the key), failures of a RELEASE leave the key "stuck".
#[derive(Debug, Default)]
pub struct StallTracker {
    pressed: BTreeSet<u16>,
}

impl StallTracker {
    pub fn note_write_result(&mut self, ok: bool, code: u16) {
        if code == EV_SYN {
            return;
        }
        if ok {
            // successful write: the visible state toggles with the event.
            // we cannot know direction here; caller passes events in order,
            // so: if code in pressed → this was a release; else a press.
            if self.pressed.contains(&code) {
                self.pressed.remove(&code);
            } else {
                self.pressed.insert(code);
            }
        }
        // failures: state believed by the OS unchanged — pressed set keeps
        // representing exactly what the OS currently sees. A stalled PRESS
        // never reached the OS (nothing to remember); a stalled RELEASE
        // leaves the key in pressed → flushed later by `take_flush`.
    }

    /// Keys the OS may currently believe are pressed (stuck after a
    /// failed release write). Consumed on recovery.
    pub fn take_flush(&mut self) -> Vec<u16> {
        let out = self.pressed.iter().copied().collect::<Vec<u16>>();
        self.pressed.clear();
        out
    }

    pub fn len(&self) -> usize {
        self.pressed.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pressed.is_empty()
    }
}

// ---------------------------------------------------------------------------
// UInputWriter (blocking writes are fine here — this runs on the SINK thread)
// ---------------------------------------------------------------------------

pub struct UInputWriter {
    fd: RawFd,
}

impl UInputWriter {
    pub fn open() -> Result<Self> {
        unsafe {
            let path_c = std::ffi::CString::new(UINPUT_PATH).unwrap();
            let fd = libc::open(
                path_c.as_ptr(),
                libc::O_WRONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
            );
            if fd < 0 {
                anyhow::bail!(
                    "open {UINPUT_PATH}: {} (input group needed?)",
                    std::io::Error::last_os_error()
                );
            }
            let ioctl = |req: u64, arg: i32| -> Result<()> {
                if libc::ioctl(fd, req as libc::c_ulong, arg) != 0 {
                    anyhow::bail!(
                        "uinput ioctl {req:#x}: {}",
                        std::io::Error::last_os_error()
                    );
                }
                Ok(())
            };
            ioctl(UI_SET_EVBIT, EV_KEY as i32)?;
            ioctl(UI_SET_EVBIT, EV_SYN as i32)?;
            for code in 0..248u16 {
                ioctl(UI_SET_KEYBIT, code as i32)?;
            }
            // uinput_user_dev setup write
            let mut buf = vec![0u8; UINPUT_USER_DEV_BYTES];
            buf[..VIRT_NAME.len()].copy_from_slice(VIRT_NAME.as_bytes());
            // input_id at offset 80: bustype=BUS_VIRTUAL(6), vendor/product/version
            let id_offset = 80usize;
            buf[id_offset] = 6;
            buf[id_offset + 2] = 1;
            buf[id_offset + 4] = 1;
            buf[id_offset + 6] = 1;
            let written = libc::write(
                fd,
                buf.as_ptr() as *const libc::c_void,
                buf.len(),
            );
            if written < 0 {
                anyhow::bail!("uinput setup write: {}", std::io::Error::last_os_error());
            }
            if libc::ioctl(fd, UI_DEV_CREATE as libc::c_ulong) != 0 {
                anyhow::bail!("UI_DEV_CREATE: {}", std::io::Error::last_os_error());
            }
            cwdebug("virtual device created");
            Ok(UInputWriter { fd })
        }
    }

    /// One raw event. Non-blocking fd: returns `io::ErrorKind::WouldBlock`
    /// on a full kernel queue (stall).
    pub fn write_event(&self, type_: u16, code: u16, value: i32) -> std::io::Result<()> {
        let ev = InputEvent {
            tv_sec: 0,
            tv_usec: 0,
            type_,
            code,
            value,
        };
        loop {
            let n = unsafe {
                libc::write(
                    self.fd,
                    &ev as *const InputEvent as *const libc::c_void,
                    std::mem::size_of::<InputEvent>(),
                )
            };
            if n >= 0 {
                return Ok(());
            }
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err); // WouldBlock when the kernel queue is full
        }
    }

    pub fn write_syn(&self) -> std::io::Result<()> {
        self.write_event(EV_SYN, SYN_REPORT, 0)
    }
}

impl Drop for UInputWriter {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.fd, UI_DEV_DESTROY as libc::c_ulong);
            libc::close(self.fd);
        }
        cwdebug("virtual device destroyed");
    }
}

fn cwdebug(msg: &str) {
    tracing::debug!(target: "mackeys::uinput", "{msg}");
}

// ---------------------------------------------------------------------------
// Sink: channel + dedicated writer thread with drop-on-stall policy
// ---------------------------------------------------------------------------

pub struct Sink {
    tx: mpsc::SyncSender<Out>,
}

static DROPPED: AtomicU64 = AtomicU64::new(0);
static STALLED: AtomicU64 = AtomicU64::new(0);

const STALL_DROP_DEADLINE_MS: u64 = 1000; // sustained stall → drop queue
const CHANNEL_CAP: usize = 4096;

impl Sink {
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<Out>(CHANNEL_CAP);
        std::thread::Builder::new()
            .name("mackeys-uinput-sink".into())
            .spawn(move || sink_loop(rx))
            .context("spawn sink thread")?;
        Ok(Sink { tx })
    }

    /// Never blocks: full channel = dropped events with changelog.
    /// (The engine delivering important events during a stall is the
    /// exception path — see StallTracker recovery flush inside the loop.)
    pub fn emit(&self, out: Out) -> Result<()> {
        if let Err(e) = self.tx.try_send(out) {
            match e {
                mpsc::TrySendError::Full(_) => {
                    let n = DROPPED.fetch_add(1, Ordering::Relaxed) + 1;
                    if n == 1 || n == 100 || n % 500 == 0 {
                        tracing::warn!(dropped = n, "sink channel full — events dropped (uinput stalled?)");
                    }
                }
                mpsc::TrySendError::Disconnected(_) => {
                    anyhow::bail!("sink thread gone");
                }
            }
        }
        Ok(())
    }
}

enum StallState {
    Streaming,
    Stalled { since: std::time::Instant, retry_q: Vec<Out> },
}

fn sink_loop(rx: mpsc::Receiver<Out>) {
    let writer = match UInputWriter::open() {
        Ok(w) => w,
        Err(e) => {
            tracing::error!(error = %e, "uinput writer unavailable — daemon lives but produces no keys");
            // main loop stays alive; heartbeat keeps diagnosing
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
    };
    let mut tracker = StallTracker::default();
    let mut st = StallState::Streaming;

    loop {
        // receive with a small timeout so stalled flushes can still recover
        let out = match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(o) => Some(o),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        match (&mut st, out) {
            (StallState::Streaming, Some(o)) => {
                if let Err(e) = process(&writer, &mut tracker, o) {
                    tracing::warn!(error = %e, "uinput stall begins — switching to drop mode (EAGAIN)");
                    st = StallState::Stalled {
                        since: std::time::Instant::now(),
                        retry_q: vec![],
                    };
                }
            }
            (StallState::Stalled { since, retry_q }, Some(o)) => {
                match process(&writer, &mut tracker, o.clone()) {
                    Ok(()) => {
                        // recovered!
                        tracing::info!(
                            stalled_ms = since.elapsed().as_millis() as u64,
                            "uinput recovered — flushing stuck releases"
                        );
                        for code in tracker.take_flush() {
                            let _ = writer.write_event(EV_KEY, code, 0);
                            let _ = writer.write_syn();
                        }
                        retry_q.clear();
                        st = StallState::Streaming;
                    }
                    Err(_) => {
                        retry_q.push(o);
                        let elapsed = since.elapsed().as_millis();
                        if elapsed as u64 > STALL_DROP_DEADLINE_MS {
                            // queue exceeded a second of stall: drop remnants;
                            // the OS may believe some keys pressed — releases
                            // flushed on recovery (tracker keeps them).
                            retry_q.clear();
                            STALLED.fetch_add(1, Ordering::Relaxed);
                            if STALLED.load(Ordering::Relaxed) % 10 == 0 {
                                tracing::error!(
                                    stalled_for_ms = elapsed as u64,
                                    "uinput still stalled — the compositor is not consuming /dev/input/event*; dropping events"
                                );
                            }
                        }
                    }
                }
            }
            (StallState::Stalled { since, retry_q }, None) => {
                if since.elapsed().as_millis() as u64 > STALL_DROP_DEADLINE_MS {
                    retry_q.clear();
                }
            }
            (StallState::Streaming, None) => {}
        }

        // retry queued events when streaming resumes only (kept in stalled
        // state until recovery clears) — nothing to do here beyond the arms.
        let _ = retry_q_marker();
    }
}

fn retry_q_marker() {}

/// Process one Out command against the writer + tracker.
/// Errors mean WouldBlock (stall) — surfaced to the caller.
fn process(writer: &UInputWriter, tracker: &mut StallTracker, out: Out) -> Result<()> {
    match out {
        Out::Key(code, value) => {
            writer.write_event(EV_KEY, code, value)?;
            writer.write_syn()?; // SYN after every key; may both fail on stall
            tracker.note_write_result(true, code);
            if value == 0 {
                // NOTE: a failed release path is recovered via tracker.take_flush()
                let _ = value;
            }
            Ok(())
        }
        Out::Sync => Ok(()), // our writer already syncs per event
        Out::ReleaseAll => {
            for code in tracker.take_flush() {
                writer.write_event(EV_KEY, code, 0)?;
                writer.write_syn()?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_flow_empty() {
        let mut t = StallTracker::default();
        t.note_write_result(true, 29);
        t.note_write_result(true, 29); // down then up — clean
        assert!(t.is_empty());
        assert_eq!(t.take_flush(), vec![]);
    }

    #[test]
    fn failed_release_marks_stuck_and_flush_claims() {
        let mut t = StallTracker::default();
        t.note_write_result(true, 29);  // down ok
        t.note_write_result(false, 29); // up FAILED — stuck
        assert_eq!(t.take_flush(), vec![29], "failed release must flush");
        assert_eq!(t.take_flush(), vec![], "flush consumed");
    }

    #[test]
    fn failed_press_is_not_remembered() {
        let mut t = StallTracker::default();
        t.note_write_result(false, 30); // press failed — OS never saw it
        assert_eq!(t.take_flush(), vec![], "failed press is not tracked");
    }

    #[test]
    fn multi_stuck_flush_order() {
        let mut t = StallTracker::default();
        t.note_write_result(true, 29);
        t.note_write_result(true, 42);
        t.note_write_result(false, 29);
        t.note_write_result(false, 42);
        assert_eq!(t.take_flush(), vec![29, 42]);
    }

    #[test]
    fn release_all_recovery_via_policy() {
        // Out::ReleaseAll path: everything tracked pressed gets released
        let mut t = StallTracker::default();
        t.note_write_result(true, 29);
        t.note_write_result(true, 42);
        let acts = vec![Out::ReleaseAll];
        let mut flushed = Vec::new();
        for o in acts {
            match o {
                Out::ReleaseAll => flushed.extend(t.take_flush()),
                _ => {}
            }
        }
        assert_eq!(flushed, vec![29, 42]);
        assert!(t.is_empty());
    }

    #[test]
    fn write_event_nonblocking_smoke() {
        // live smoke: needs /dev/uinput; runs on the owner's machine, skips elsewhere
        if !std::path::Path::new("/dev/uinput").exists() {
            return;
        }
        let w = match UInputWriter::open() {
            Ok(w) => w,
            Err(_) => return,
        };
        // press + release of a harmless key, SYN-flushed
        w.write_event(EV_KEY, 41, 1).unwrap();
        w.write_syn().unwrap();
        w.write_event(EV_KEY, 41, 0).unwrap();
        w.write_syn().unwrap();
    }
}
