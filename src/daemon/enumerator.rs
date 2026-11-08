//! Background device enumerator. Spec: slow FS/evdev IO must NEVER run on
//! the engine thread — incident 2026-11-08: scan() inside the main loop
//! blocked it for ~16 min and the whole keyboard went mute.

use crate::daemon::devices;
use crate::daemon::input;
use crate::daemon::input::LoopMsg;
use crate::types::Kb;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};

pub struct Enumerator {
    tx: mpsc::Sender<LoopMsg>,
    excludes: Vec<String>,
    per_device_kb: BTreeMap<String, u8>,
    default_kb: Kb,
    default_swap: bool,
    tag_next: AtomicUsize,
    spawned: Mutex<BTreeMap<usize, PathBuf>>,
    /// shared registry read by the engine loop for per-device kbtype
    pub device_kbs: Arc<Mutex<BTreeMap<u8, (Kb, bool)>>>,
    busy: AtomicBool,
}

impl Enumerator {
    pub fn new(
        tx: mpsc::Sender<LoopMsg>,
        excludes: Vec<String>,
        per_device_kb: BTreeMap<String, u8>,
        default_kb: Kb,
        default_swap: bool,
        device_kbs: Arc<Mutex<BTreeMap<u8, (Kb, bool)>>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            tx,
            excludes,
            per_device_kb,
            default_kb,
            default_swap,
            tag_next: AtomicUsize::new(0),
            spawned: Mutex::new(BTreeSet::new().into_iter().collect::<BTreeMap<_, _>>()),
            device_kbs,
            busy: AtomicBool::new(false),
        })
    }

    /// One enumeration pass. Coalesced by `busy` — concurrent requests are
    /// dropped; later device events / the 60 s fallback re-trigger already.
    pub fn scan_once(self: &Arc<Self>) -> usize {
        if self.busy.swap(true, Ordering::Relaxed) {
            return 0;
        }
        let mut grabbed = 0usize;
        match devices::scan(&self.excludes) {
            Ok(infos) => {
                for i in infos.iter().filter(|i| i.managed) {
                    let fresh = {
                        let mut sp = self.spawned.lock().unwrap();
                        if sp.values().any(|p| p == &i.path) {
                            None
                        } else {
                            let tag = self.tag_next.fetch_add(1, Ordering::Relaxed);
                            sp.insert(tag, i.path.clone());
                            Some(tag)
                        }
                    };
                    if let Some(tag) = fresh {
                        match input::spawn_reader(
                            i.clone(),
                            tag,
                            self.tx.clone(),
                            Arc::new(AtomicBool::new(false)), // per-reader lives until node death
                        ) {
                            Ok(()) => {
                                tracing::info!(name = %i.name, path = %i.path.display(), "grabbed");
                                let kb_byte = self
                                    .per_device_kb
                                    .get(&i.name)
                                    .copied()
                                    .unwrap_or_else(|| match self.default_kb {
                                        Kb::Apple => 1,
                                        _ => 0,
                                    });
                                let dev_kb = match kb_byte {
                                    1 => Kb::Apple,
                                    _ => Kb::Windows,
                                };
                                self.device_kbs
                                    .lock()
                                    .unwrap()
                                    .insert(tag as u8, (dev_kb, self.default_swap));
                                grabbed += 1;
                            }
                            Err(e) => {
                                tracing::debug!(name = %i.name, error = %e, "hotplug grab refused");
                                self.spawned.lock().unwrap().remove(&tag);
                            }
                        }
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "device scan failed — will retry on next trigger"),
        }
        self.busy.store(false, Ordering::Relaxed);
        grabbed
    }

    /// Reader died → release its bookkeeping; the path may respawn on the
    /// next enumeration pass.
    pub fn release_tag(&self, tag: usize) {
        let gone = self.spawned.lock().unwrap().remove(&tag);
        self.device_kbs.lock().unwrap().remove(&(tag as u8));
        if let Some(path) = gone {
            tracing::info!(tag, path = %path.display(), "device reader ended; state flushed");
        } else {
            tracing::info!(tag, "device reader ended; state flushed");
        }
    }
}

#[allow(unused)]
fn _keep_atomicbool_import(hint: &'static str) -> AtomicBool {
    tracing::debug!(msg = hint);
    AtomicBool::new(false)
}
