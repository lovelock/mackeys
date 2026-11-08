//! uinput sink: one virtual keyboard named `mackeys virtual keyboard`.

use crate::types::Out;
use anyhow::{Context, Result};
use evdev::{
    uinput::VirtualDevice, uinput::VirtualDeviceBuilder, AttributeSet, EventType, InputEvent,
    KeyCode,
};

pub struct Sink {
    virt: VirtualDevice,
}

impl Sink {
    pub fn new() -> Result<Self> {
        // full EV_KEY range (0..248) so output can express ANY mapped key
        let mut keys = AttributeSet::<KeyCode>::new();
        for code in 0..248u16 {
            keys.insert(KeyCode(code));
        }
        let virt = VirtualDeviceBuilder::new()?
            .name(b"mackeys virtual keyboard")
            .with_keys(&keys)?
            .build()
            .context("build virtual device (check /dev/uinput perms)")?;
        Ok(Sink { virt })
    }

    /// Emit one Out command; keys accumulate, `Sync` flushes a batch.
    pub fn emit(&mut self, out: Out) -> Result<()> {
        match out {
            Out::Key(code, value) => {
                let ev = InputEvent::new(EventType::KEY.0, code, value);
                self.virt.emit(&[ev])?;
            }
            Out::Sync => { /* every Key already emits with the evdev crate's emit(); Syn is implied */ }
            Out::ReleaseAll => { /* teardown writes its own releases; nothing extra here */ }
        }
        Ok(())
    }
}
