//! Daemon loop: input readers → engine → uinput sink, plus lifecycle.

pub mod devices;
pub mod input;
mod sink;

use input::LoopMsg;
use crate::adapters::FocusSource;
use std::sync::mpsc::{SyncSender, Sender};
use sink::Sink;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use crate::engine::{apply, shutdown, DeviceState};
use crate::types::{Act, Ctx, Kb, Out, RuleSet};

/// Atomic<Ctx> carrier for the event loop.
#[derive(Default)]
pub struct AtomicCtx(pub std::sync::atomic::AtomicU8);
impl AtomicCtx {
    pub fn get(&self) -> Ctx {
        match self.0.load(Ordering::Relaxed) { 1 => Ctx::Terminal, _ => Ctx::Gui }
    }
    pub fn set(&self, c: Ctx) {
        self.0.store(match c { Ctx::Terminal => 1, _ => 0 }, Ordering::Relaxed);
    }
}

/// Administrative entry for the `run` subcommand.
pub fn run(args: &[String]) -> anyhow::Result<()> {
    // config load; absent file → whole-defaults (owner prefs baked into code)
    let cfg = match crate::map::config::load_config(&crate::setup::config_path()?) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error=%e, "config not loaded — using defaults (run `mackeys setup`)");
            crate::map::config::Config::parse_toml("")?
        }
    };
    if args.iter().any(|a| a == "--dry") {
        let infos = devices::scan(&[])?;
        println!("mackeys dry-run: device inventory:");
        for i in &infos {
            println!(
                "  {:10} letter-device={} {}",
                if i.managed { "[managed]" } else { "[native ]" },
                i.letter.arg_if(),
                i.name,
            );
        }
        println!("(no grabs performed in dry mode)");
        return Ok(());
    }
    let kb = match cfg.general.kbtype_default.as_str() {
        "apple" => Kb::Apple,
        _ => Kb::Windows,
    };
    let swap = cfg.general.swap_caps_lctrl;
    let (gui, term) = crate::map::compile_all(kb, &cfg);
    let per_device_kb: std::collections::BTreeMap<String, u8> = cfg
        .per_device
        .iter()
        .map(|(name, kind)| (
            name.clone(),
            match kind.as_str() { "apple" => 1, _ => 0 },
        ))
        .collect();
    run_daemon(
        gui,
        term,
        cfg.targets.terminal_classes.clone(),
        cfg.general.adapter.clone(),
        kb,
        swap,
        per_device_kb,
        cfg.general.exclude_devices.clone(),
    )
}

trait BoolExt {
    fn arg_if(&self) -> &'static str;
}
impl BoolExt for bool {
    fn arg_if(&self) -> &'static str {
        if *self { "yes" } else { "no " }
    }
}

pub fn run_daemon(
    rules_gui: RuleSet,
    rules_term: RuleSet,
    terminal_classes: Vec<String>,
    adapter_name: String,
    default_kb: Kb,
    default_swap: bool,
    per_device_kb: std::collections::BTreeMap<String, u8>,   // device name → kbtype byte
    excludes: Vec<String>,
) -> anyhow::Result<()> {
    tracing::info!("mackeys daemon starting");
    let (tx, rx) = mpsc::channel::<LoopMsg>();
    let stop = Arc::new(AtomicBool::new(false));
    let ctx = Arc::new(AtomicCtx::default());

    // signal handling: SIGINT/SIGTERM set the stop flag; main loop observes it.
    signal_hook::flag::register(signal_hook::consts::SIGINT, stop.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, stop.clone())?;

    // focus adapter (replaceable component, spec §4).
    // INSTALL MUST NOT KILL THE DAEMON (incident 2026-11-08: a boot-time
    // KWin-not-ready window crash-looped the unit 19× and left the keyboard
    // unmapped). Install retries in the background thread; key remapping
    // runs immediately regardless of focus-report state.
    let adapter: std::sync::Arc<dyn FocusSource> =
        std::sync::Arc::from(crate::adapters::best_for(&adapter_name)?);
    tracing::info!("focus adapter: {}", adapter.id());
    {
        // bridge: focus reports → our loop. The relay thread may exist before
        // the adapter watches — it just waits on the channel meanwhile.
        let (sender, recvr) = mpsc::sync_channel::<Option<crate::adapters::WindowReport>>(16);
        let i_tx = tx.clone();
        std::thread::Builder::new()
            .name("mackeys-focus-relay".into())
            .spawn(move || {
                for report in recvr {
                    if i_tx.send(LoopMsg::Focus(report)).is_err() {
                        break;
                    }
                }
            })
            .ok();

        // installer retry loop: success → hand the sender to spawn_watch
        let a = adapter.clone();
        std::thread::Builder::new()
            .name("mackeys-adapter-install".into())
            .spawn(move || {
                let mut delay = 2u64;
                loop {
                    match a.install() {
                        Ok(()) => break,
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "adapter install failed — key remap already live; retrying in {delay}s"
                            );
                            std::thread::sleep(std::time::Duration::from_secs(delay));
                            delay = (delay * 2).min(30);
                        }
                    }
                }
                if let Err(e) = a.spawn_watch(sender) {
                    tracing::error!(error = %e, "adapter watch failed after install");
                }
            })
            .ok();
    }

    // spawn readers for managed devices
    let infos = devices::scan(&excludes)?;
    let mut device_kbs = std::collections::BTreeMap::<u8, (Kb, bool)>::new();
    let mut spawned: std::collections::BTreeMap<usize, std::path::PathBuf> = Default::default();
    let mut tag_next = 0usize;
    for i in &infos {
        if !i.managed {
            continue;
        }
        match input::spawn_reader(i.clone(), tag_next, tx.clone(), stop.clone()) {
            Ok(_) => tracing::info!(name = %i.name, path = %i.path.display(), "grabbed"),
            Err(e) => tracing::warn!(name = %i.name, error = %e, "grab failed — device stays native"),
        }
        let kb_byte = per_device_kb.get(&i.name).copied().unwrap_or_else(|| match default_kb { Kb::Apple => 1, _ => 0 });
        let dev_kb = match kb_byte { 1 => Kb::Apple, _ => Kb::Windows };
        device_kbs.insert(tag_next as u8, (dev_kb, default_swap));
        spawned.insert(tag_next, i.path.clone());
        tag_next += 1;
    }
    let tx_rescan = tx.clone();
    drop(tx);

    let mut sink = Sink::new()?;
    let mut states: BTreeMap<usize, DeviceState> = BTreeMap::new();
    let terminal_classes_ref: std::sync::RwLock<Vec<String>> =
        std::sync::RwLock::new(terminal_classes.clone());
    let mut sigtick = 0;

    while !stop.load(Ordering::Relaxed) {
        let msg = match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(m) => m,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // hotplug: low-frequency rescan grabs newly appeared devices
                rescan_tick(&tx_rescan, &stop, &mut spawned, &mut tag_next, &excludes, &mut device_kbs, &per_device_kb, default_kb, default_swap);
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match msg {
            LoopMsg::Focus(report) => {
                let term = report
                    .as_ref()
                    .map(|r| crate::adapters::is_terminal(r, &terminal_classes_ref.read().unwrap()))
                    .unwrap_or(false);
                tracing::debug!(
                    app = report.as_ref().map(|r| format!("{} / {}", r.resource_class, r.resource_name)).unwrap_or_default(),
                    is_terminal = term,
                    "FOCUS-report"
                );
                let newctx = if term { Ctx::Terminal } else { Ctx::Gui };
                if ctx.get() != newctx {
                    // context change: tear down held combos (spec §8 race rule)
                    for (tag, st) in states.iter_mut() {
                        let outv = crate::engine::release_held(st);
                        tracing::debug!(tag, cmds = outv.len(), "context switch teardown");
                        emit(&mut sink, outv);
                    }
                    let app = report.as_ref().map(|r| r.resource_class.clone()).unwrap_or_default();
                    tracing::info!(kind = if newctx == Ctx::Terminal { "terminal" } else { "gui" }, app, "context change");
                    ctx.set(newctx);
                }
            }
            LoopMsg::Act(tag, act) => {
                let ctx_now = ctx.get();
                let rules = match ctx_now {
                    Ctx::Terminal => &rules_term,
                    _ => &rules_gui,
                };
                let state = states.entry(tag).or_default();
                let (kb, swap) = device_kbs.get(&(tag as u8)).copied().unwrap_or((default_kb, default_swap));
                let outv = apply(state, act, kb, swap, ctx_now, rules);
                emit(&mut sink, outv);
            }
            LoopMsg::Eof(tag) => {
                if let Some(mut st) = states.remove(&tag) {
                    emit(&mut sink, shutdown(&mut st));
                }
                states.remove(&tag);
                spawned.remove(&tag);
                device_kbs.remove(&(tag as u8));
                tracing::info!(tag, "device reader ended; state flushed");
            }
        }
        // safety valve: observe stop even under busy traffic
        sigtick += 1;
        if sigtick % 500 == 0 && stop.load(Ordering::Relaxed) {
            break;
        }
    }

    // teardown: shutdown every device state, flush, then exit (readers ungrab)
    stop.store(true, Ordering::Relaxed);
    for (tag, st) in states.iter_mut() {
        let outv = shutdown(st);
        tracing::debug!(tag, cmds = outv.len(), "teardown");
        emit(&mut sink, outv);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn rescan_tick(
    tx: &mpsc::Sender<input::LoopMsg>,
    stop: &Arc<AtomicBool>,
    spawned: &mut std::collections::BTreeMap<usize, std::path::PathBuf>,
    tag_next: &mut usize,
    excludes: &[String],
    device_kbs: &mut std::collections::BTreeMap<u8, (Kb, bool)>,
    per_device_kb: &std::collections::BTreeMap<String, u8>,
    default_kb: Kb,
    default_swap: bool,
) {
    let Ok(infos) = devices::scan(excludes) else { return };
    for i in infos.iter().filter(|i| i.managed) {
        if spawned.values().any(|p| p == &i.path) {
            continue;
        }
        let tag = *tag_next;
        *tag_next += 1;
        match input::spawn_reader(i.clone(), tag, tx.clone(), stop.clone()) {
            Ok(()) => {
                tracing::info!(name = %i.name, path = %i.path.display(), "hotplug: grabbed");
                let kb_byte = per_device_kb.get(&i.name).copied().unwrap_or_else(|| match default_kb { Kb::Apple => 1, _ => 0 });
                let dev_kb = match kb_byte { 1 => Kb::Apple, _ => Kb::Windows };
                device_kbs.insert(tag as u8, (dev_kb, default_swap));
                spawned.insert(tag, i.path.clone());
            }
            Err(e) => {
                // un-spawn (user velho devices without grab stay native; untracked)
                tracing::debug!(name = %i.name, error = %e, "hotplug grab refused");
            }
        }
    }
}

fn emit(sink: &mut Sink, outv: Vec<Out>) {
    for o in outv {
        if let Err(e) = sink.emit(o) {
            tracing::warn!(error = %e, "sink emit failed");
        }
    }
}
