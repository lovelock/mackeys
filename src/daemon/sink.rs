//! uinput sink — faithful port of xwaykeyz `output.py`:
//!   * `evdev` crate's `VirtualDevice` (same protocol implementation family as
//!     python-evdev's UInput, which Toshy uses),
//!   * the SAME capability set (all keys minus BTN, then mouse/touchpad
//!     buttons added back; EV_REL axes 0,1,6,8,9),
//!   * the SAME fd flags: `O_RDWR | O_NONBLOCK` (python-evdev's
//!     `_uinput.c` opens exactly with these — this is why Toshy never
//!     hard-blocks the daemon on a full kernel uinput buffer),
//!   * synchronous writes with SYN after every key action (`send_key_action_fast`).
//!
//! Where Toshy would let a write error propagate (crash/exception), we
//! degrade gracefully: WouldBlock → drop that single event with a rate-limited
//! warning. No sink threads, no custom ioctl code — 不做发明.

use anyhow::{Context, Result};
use evdev::{
    uinput::VirtualDevice, uinput::VirtualDeviceBuilder, AttributeSet, EventType, InputEvent,
    KeyCode, RelativeAxisCode,
};

pub const VIRT_NAME: &str = "mackeys virtual keyboard";

pub struct Sink {
    virt: VirtualDevice,
    dropped: u64,
}

impl Sink {
    pub fn new() -> Result<Self> {
        // capability set — mirror of xwaykeyz output.py:
        //   _KEYBOARD_KEYS = keys - BTN + mouse buttons + touchpad buttons
        let mut keys = AttributeSet::<KeyCode>::new();
        for code in 1u16..=0x2ff {
            let is_btn = (0x100..=0x151).contains(&code) || (0x2c0..=0x2ff).contains(&code);
            if !is_btn {
                keys.insert(KeyCode(code));
            }
        }
        for code in [
            256u16, 257, 258, 259, 260, 261, 262, 263, 264, 265, // BTN_0..BTN_9
            272, 273, 274, 275, 276, // mouse buttons
            325, 330, 333, 334, 335, 336, 337, 338, 339, // touchpad buttons
        ] {
            keys.insert(KeyCode(code));
        }
        // EV_REL: 0=REL_X 1=REL_Y 6=REL_HWHEEL 8=REL_WHEEL 9=REL_MISC
        let mut rel = AttributeSet::<RelativeAxisCode>::new();
        for code in [0u16, 1, 6, 8, 9] {
            rel.insert(RelativeAxisCode(code));
        }

        let virt = VirtualDeviceBuilder::new()?
            .name(VIRT_NAME.as_bytes())
            .with_keys(&keys)?
            .with_relative_axes(&rel)?
            .build()
            .context("build virtual device (check /dev/uinput perms)")?;

        // mirror python-evdev `_uinput.c`: open flags O_RDWR | O_NONBLOCK.
        // The crate opens blocking; flip the fd non-blocking exactly like
        // Toshy's stack does, so a full kernel buffer never wedges us.
        unsafe {
            use std::os::fd::AsRawFd;
            let fd = virt.as_raw_fd();
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }

        tracing::debug!(target: "mackeys::uinput", "virtual device created");
        Ok(Sink { virt, dropped: 0 })
    }

    /// Synchronous write, one event + SYN — same shape as
    /// `send_key_action_fast()`: write(EV_KEY, key, action) then syn().
    /// WouldBlock (full kernel buffer / stalled consumer) → drop the event.
    /// xwaykeyz keeps 1 ms pre/post silence between key actions on purpose
    /// (`_THROTTLE_MIN_PRE_MS/_POST_MS`): "having actual zero delays between
    /// key action events causes too many problems, may be partly responsible
    /// for some of the stuck key issues some users run into."
    const MIN_PRE: std::time::Duration = std::time::Duration::from_millis(1);
    const MIN_POST: std::time::Duration = std::time::Duration::from_millis(1);

    pub fn emit(&mut self, code: u16, value: i32) -> Result<()> {
        std::thread::sleep(Self::MIN_PRE);
        let ev = InputEvent::new(EventType::KEY.0, code, value);
        let res = self.virt.emit(&[ev]);
        match &res {
            Ok(()) => tracing::trace!(code, value, "uinput write ok"),
            Err(e) => tracing::debug!(code, value, error = %e, "uinput write result"),
        }
        std::thread::sleep(Self::MIN_POST);
        match res {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                self.dropped += 1;
                if self.dropped == 1 || self.dropped == 100 || self.dropped % 1000 == 0 {
                    tracing::warn!(
                        dropped = self.dropped,
                        "uinput buffer full (compositor stalled?) — dropping key events"
                    );
                }
                Ok(())
            }
            Err(e) => {
                tracing::error!(error = %e, "uinput write failed");
                Ok(()) // never let output kill the engine loop
            }
        }
    }

    /// Teardown: release everything tracked by the engine (engine emits the
    /// individual releases; this is just a final SYN flush).
    pub fn flush(&mut self) -> Result<()> {
        let ev = InputEvent::new(EventType::SYNCHRONIZATION.0, 0, 0);
        let _ = self.virt.emit(&[ev]);
        Ok(())
    }
}
