//! KWin adapter: embedded JS script + dbus serve for NotifyActiveWindow.

use super::WindowReport;
use anyhow::{Context, Result};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

pub const BUS_NAME: &str = "org.mackeys.Daemon";
pub const OBJ_PATH: &str = "/org/mackeys/Daemon";
pub const IFACE_NAME: &str = "org.mackeys.Daemon";

pub const SCRIPT_ID: &str = "mackeys-focus";
pub const SCRIPT_NAME_META: &str = include_str!("kwin_script/metadata.json");
pub const SCRIPT_MAIN: &str = include_str!("kwin_script/main.js");

pub struct KwinAdapter {
    current: Arc<Mutex<Option<WindowReport>>>,
}

impl KwinAdapter {
    pub fn new() -> Self {
        KwinAdapter {
            current: Arc::new(Mutex::new(None)),
        }
    }
}

#[derive(Clone)]
pub struct Notifier {
    tx: SyncSender<Option<WindowReport>>,
    current: Arc<Mutex<Option<WindowReport>>>,
}

#[zbus::interface(name = "org.mackeys.Daemon")]
impl Notifier {
    async fn notify_active_window(
        &self,
        caption: String,
        resource_class: String,
        resource_name: String,
    ) {
        let report = WindowReport {
            caption,
            resource_class,
            resource_name,
        };
        *self.current.lock().unwrap() = Some(report.clone());
        let _ = self.tx.send(Some(report));
    }
}

impl KwinAdapter {
    fn script_dir(&self) -> Result<std::path::PathBuf> {
        let home = std::env::var("HOME").context("no HOME")?;
        let dir = std::path::PathBuf::from(home)
            .join(".local/share/kwin/scripts")
            .join(SCRIPT_ID);
        Ok(dir)
    }

    fn kwin_dbus_call(&self, method: &str, arg: Option<&str>) -> Result<Option<String>> {
        use zbus::blocking::Connection;
        let con = Connection::session()?;
        // KWin's scripting object exposes org.kde.kwin.Scripting on /Scripting
        let svc = if method.starts_with("reconfigure") {
            ("org.kde.KWin", "/KWin", "org.kde.KWin")
        } else {
            ("org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting")
        };
        let (svc, path, iface) = (svc.0, svc.1, svc.2);
        let msg = match arg {
            Some(a) => format!(
                "busctl call --user {svc} {path} {iface} {method} s \"{a}\""
            ),
            None => format!("busctl call --user {svc} {path} {iface} {method}"),
        };
        let out = std::process::Command::new("sh").arg("-c").arg(&msg).output()?;
        if !out.status.success() {
            anyhow::bail!(
                "kwin dbus {method} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()))
    }
}

impl super::FocusSource for KwinAdapter {
    fn id(&self) -> &'static str {
        "kwin"
    }
    fn install(&self) -> Result<()> {
        let dir = self.script_dir()?;
        let code_dir = dir.join("contents/code");
        std::fs::create_dir_all(&code_dir)?;
        std::fs::write(dir.join("metadata.json"), SCRIPT_NAME_META)?;
        std::fs::write(code_dir.join("main.js"), SCRIPT_MAIN)?;

        // Enable persistently (Toshy knowledge: dbus loadScript alone doesn't
        // START the script; kwinrc Plugins.<id>Enabled=true + reconfigure does)
        for kw in ["kwriteconfig6", "kwriteconfig5"] {
            let _ = std::process::Command::new(kw)
                .args(["--file", "kwinrc", "--group", "Plugins", "--key", "mackeysFocusEnabled", "true"])
                .output();
        }
        // dbus loadScript(s) is a path-based no-op (review C3): kwinrc
        // Enabled=true + reconfigure is the load path that actually works
        let _ = self.kwin_dbus_call("reconfigure", None);
        for _ in 0..5 {
            std::thread::sleep(std::time::Duration::from_millis(700));
            let loaded = self.kwin_dbus_call("isScriptLoaded", Some(SCRIPT_ID))?;
            if loaded.map(|s| s.to_lowercase().contains("true")).unwrap_or(false) {
                tracing::info!("kwin adapter: script installed & loaded ({}).", SCRIPT_ID);
                return Ok(());
            }
        }
        anyhow::bail!("kwin script installed but did not report loaded — may need a relogin")
    }
    fn uninstall(&self) -> Result<()> {
        let _ = self.kwin_dbus_call("unloadScript", Some(SCRIPT_ID));
        for kw in ["kwriteconfig6", "kwriteconfig5"] {
            let _ = std::process::Command::new(kw)
                .args(["--file", "kwinrc", "--group", "Plugins", "--key", "mackeysFocusEnabled", "false"])
                .output();
        }
        let dir = self.script_dir()?;
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        tracing::info!("kwin adapter: script unloaded & removed");
        Ok(())
    }
    fn current(&self) -> Option<WindowReport> {
        self.current.lock().unwrap().clone()
    }
    fn spawn_watch(self: Arc<Self>, tx: SyncSender<Option<WindowReport>>) -> Result<()> {
        let notifier = Notifier {
            tx,
            current: self.current.clone(),
        };
        std::thread::Builder::new()
            .name("mackeys-kwin-dbus".into())
            .spawn(move || {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!(error=%e, "no tokio runtime for kwin adapter");
                        return;
                    }
                };
                rt.block_on(async move {
                    let builder = match zbus::connection::Builder::session() {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::error!(error=%e, "session bus unavailable");
                            return;
                        }
                    };
                    let built = builder
                        .name(BUS_NAME)
                        .expect("bus name taken — another mackeys daemon running?")
                        .serve_at(OBJ_PATH, notifier)
                        .expect("serve interface")
                        .build();
                    let con = built.await;
                    let con = match con {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(error=%e, "kwin adapter: dbus setup failed");
                            return;
                        }
                    };
                    // vocabulary: event loop lives while the connection is held
                    tokio::signal::ctrl_c().await.ok();
                    drop(con);
                });
            })?;
        Ok(())
    }
}
