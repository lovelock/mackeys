//! Daemon loop: input readers → engine → uinput sink, lifecycle, focus.
//! The main loop is deliberately THIN: routing events only. Any slow work
//! (device enumeration, adapter install) lives in background threads —
//! incident 2026-11-08: a 16-minute main-loop block came from scan().

pub mod devices;
pub mod enumerator;
pub mod input;
pub mod sink;
pub mod udevwatch;

use enumerator::Enumerator;
use input::LoopMsg;
use sink::Sink;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use crate::engine::{apply, shutdown, DeviceState};
use crate::types::{Act, Ctx, Kb, Out, RuleSet};

/// Atomic<Ctx> carrier for the event loop.
#[derive(Default)]
pub struct AtomicCtx(pub std::sync::atomic::AtomicU8);
impl AtomicCtx {
    pub fn get(&self) -> Ctx {
        match self.0.load(Ordering::Relaxed) {
            1 => Ctx::Terminal,
            _ => Ctx::Gui,
        }
    }
    pub fn set(&self, c: Ctx) {
        self.0.store(match c { Ctx::Terminal => 1, _ => 0 }, Ordering::Relaxed);
    }
}

const SCAN_FALLBACK_SECS: u64 = 60; // belt & suspenders when netlink is deaf
const HEARTBEAT_SECS: u64 = 100; // wall-clock based on purpose

/// Administrative entry for the `run` subcommand.
pub fn run(args: &[String]) -> anyhow::Result<()> {
    let cfg = match crate::map::config::load_config(&crate::setup::config_path()?) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error=%e, "config not loaded — using defaults (run `mackeys setup`)");
            crate::map::config::Config::parse_toml("")?
        }
    };
    if args.iter().any(|a| a == "--dry") {
        let infos = devices::scan(&cfg.general.exclude_devices)?;
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
    let per_device_kb: BTreeMap<String, u8> = cfg
        .per_device
        .iter()
        .map(|(name, kind)| (name.clone(), match kind.as_str() { "apple" => 1, _ => 0 }))
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

/// NOTE: `per_device_kb` keys are device names; values: 1=apple 0=windows.
#[allow(clippy::too_many_arguments)]
pub fn run_daemon(
    rules_gui: RuleSet,
    rules_term: RuleSet,
    terminal_classes: Vec<String>,
    adapter_name: String,
    default_kb: Kb,
    default_swap: bool,
    per_device_kb: BTreeMap<String, u8>,
    excludes: Vec<String>,
) -> anyhow::Result<()> {
    tracing::info!("mackeys daemon starting");
    let (tx, rx) = mpsc::channel::<LoopMsg>();
    let stop = Arc::new(AtomicBool::new(false));
    let ctx = Arc::new(AtomicCtx::default());
    let cur_report: Arc<Mutex<Option<crate::adapters::WindowReport>>> =
        Arc::new(Mutex::new(None));

    // signal handling: SIGINT/SIGTERM set the stop flag; main loop observes it.
    signal_hook::flag::register(signal_hook::consts::SIGINT, stop.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, stop.clone())?;

    // device_kbs shared with the background enumerator
    let device_kbs: Arc<Mutex<BTreeMap<u8, (Kb, bool)>>> = Arc::new(Mutex::new(Default::default()));
    let enumerator = Enumerator::new(
        tx.clone(),
        excludes.clone(),
        per_device_kb.clone(),
        default_kb,
        default_swap,
        device_kbs.clone(),
    );

    // kernel uevent watcher — REAL device add/remove events
    crate::daemon::udevwatch::spawn_watcher(tx.clone(), stop.clone())?;

    // focus adapter (replaceable component, spec §4). Never fatal (incident:
    // boot-time KWin-not-ready crash-loop left the keyboard unmapped).
    let adapter: Arc<dyn crate::adapters::FocusSource> =
        Arc::from(crate::adapters::best_for(&adapter_name)?);
    tracing::info!("focus adapter: {}", adapter.id());
    {
        let a = adapter.clone();
        let stop2 = stop.clone();
        let i_tx2 = tx.clone();
        std::thread::Builder::new()
            .name("mackeys-adapter-install".into())
            .spawn(move || {
                let mut delay = 2u64;
                while !stop2.load(Ordering::Relaxed) {
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
                if stop2.load(Ordering::Relaxed) { return; }
                let (sender, recvr) =
                    mpsc::sync_channel::<Option<crate::adapters::WindowReport>>(16);
                std::thread::Builder::new()
                    .name("mackeys-focus-relay".into())
                    .spawn(move || {
                        for report in recvr {
                            if i_tx2.send(LoopMsg::Focus(report)).is_err() {
                                break;
                            }
                        }
                    })
                    .ok();
                if let Err(e) = a.spawn_watch(sender) {
                    tracing::error!(error = %e, "adapter watch failed after install");
                }
            })
            .ok();
    }

    // initial enumeration runs in the background too (never blocks the loop)
    {
        let en = enumerator.clone();
        std::thread::Builder::new()
            .name("mackeys-enum-init".into())
            .spawn(move || { en.scan_once(); })
            .ok();
    }

    let mut sink = Sink::new()?;
    let mut states: BTreeMap<usize, DeviceState> = BTreeMap::new();
    let mut sigtick = 0u64;
    let mut next_heartbeat = Instant::now() + Duration::from_secs(HEARTBEAT_SECS);
    let mut next_scan_fallback = Instant::now() + Duration::from_secs(SCAN_FALLBACK_SECS);
    let mut acts_since_tick = 0u64;
    let mut scan_fallback_count = 0u64;

    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        let timeout = std::cmp::min(next_heartbeat, next_scan_fallback)
            .checked_duration_since(now)
            .unwrap_or(Duration::from_millis(5));
        let msg = match rx.recv_timeout(timeout) {
            Ok(m) => m,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // wall-clock deadlines: fallback scan + heartbeat. The engine
                // loop handles no IO itself; scans are always background.
                if now >= next_scan_fallback {
                    next_scan_fallback = now + Duration::from_secs(SCAN_FALLBACK_SECS);
                    scan_fallback_count += 1;
                    tracing::debug!(count = scan_fallback_count, "fallback rescan");
                    let en = enumerator.clone();
                    std::thread::Builder::new()
                        .name("mackeys-enum-fallback".into())
                        .spawn(move || { en.scan_once(); })
                        .ok();
                }
                if now >= next_heartbeat {
                    next_heartbeat = now + Duration::from_secs(HEARTBEAT_SECS);
                    tracing::info!(
                        keys = acts_since_tick,
                        threads = std::fs::read_dir("/proc/self/task").map(|d| d.count()).unwrap_or_default(),
                        "heartbeat (100s wall-clock; keys>0 means events ARE arriving)"
                    );
                    acts_since_tick = 0;
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        match msg {
            LoopMsg::Rescan => {
                // IMMEDIATE background enumeration on device events (the
                // coalescing latch inside Enumerator absorbs event floods).
                // This is the path netlink uses — never gate it on deadlines
                // (bug found 2026-11-08 22:45: replug events fell into the
                // no-op branch and devices stayed ungrabbed).
                let en = enumerator.clone();
                std::thread::Builder::new()
                    .name("mackeys-enum".into())
                    .spawn(move || { en.scan_once(); })
                    .ok();
                next_scan_fallback = now + Duration::from_secs(SCAN_FALLBACK_SECS);
            }
            LoopMsg::Focus(report) => {
                if let Some(r) = &report {
                    *cur_report.lock().unwrap() = Some(r.clone());
                }
                let term = report
                    .as_ref()
                    .map(|r| crate::adapters::is_terminal(r, &terminal_classes));
                tracing::debug!(
                    app = report.as_ref().map(|r| format!("{} / {}", r.resource_class, r.resource_name)).unwrap_or_default(),
                    is_terminal = ?term,
                    "FOCUS-report"
                );
                let newctx = if term.unwrap_or(false) { Ctx::Terminal } else { Ctx::Gui };
                if ctx.get() != newctx {
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
                acts_since_tick += 1;
                let ctx_now = ctx.get();
                let rules = match ctx_now { Ctx::Terminal => &rules_term, _ => &rules_gui };
                let state = states.entry(tag).or_default();
                let (kb, swap) = device_kbs
                    .lock()
                    .unwrap()
                    .get(&(tag as u8))
                    .copied()
                    .unwrap_or((default_kb, default_swap));
                let cur_app = cur_report
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|r| r.resource_class.clone())
                    .or_else(|| cur_report.lock().unwrap().as_ref().map(|r| r.resource_name.clone()));
                let outv = apply(state, act, kb, swap, ctx_now, rules, cur_app.as_deref());
                emit(&mut sink, outv);
            }
            LoopMsg::Eof(tag) => {
                if let Some(mut st) = states.remove(&tag) {
                    emit(&mut sink, shutdown(&mut st));
                }
                states.remove(&tag);
                enumerator.release_tag(tag);
            }
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

fn emit(sink: &mut Sink, outv: Vec<Out>) {
    for o in outv {
        if let Out::Key(code, value) = o {
            let _ = sink.emit(code, value);
        }
    }
}

#[allow(dead_code)]
enum _Keep {
    _ActMarker(usize, Act),
}
