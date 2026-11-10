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

fn kpackagetool_cmd() -> Option<&'static str> {
    ["kpackagetool6", "kpackagetool5"]
        .iter()
        .find(|c| std::process::Command::new(c).arg("--help").output().map(|o| o.status.success() || true).unwrap_or(false))
        .copied()
}

fn kwriteconfig_cmd() -> &'static str {
    if std::process::Command::new("kwriteconfig6").arg("--help").output().is_ok() {
        "kwriteconfig6"
    } else {
        "kwriteconfig5"
    }
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

    /// Enquiry interface: what did the last focus report look like (faecal?
    /// for doctor and user-side diagnostics: focus a window, run busctl).
    async fn get_active_window(&self) -> (String, String, String) {
        let report = self.current.lock().unwrap().clone();
        match report {
            Some(r) => (r.caption, r.resource_class, r.resource_name),
            None => ("NO_DATA".into(), "NO_DATA".into(), "NO_DATA".into()),
        }
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

    /// Ground-truth loaded check (shared with doctor via pure query).
    pub fn script_loaded(&self) -> bool {
        matches!(
            self.kwin_dbus_call("isScriptLoaded", Some(SCRIPT_ID))
                .ok()
                .and_then(|s| s.map(|out| out.to_lowercase().contains("true"))),
            Some(true)
        )
    }
}

impl super::FocusSource for KwinAdapter {
    fn id(&self) -> &'static str {
        "kwin"
    }
    fn install(&self) -> Result<()> {
        // Faithful port of toshy_kwin_script_setup.py:
        //   1. zip contents/code/main.js + metadata.json into <name>.kwinscript
        //   2. kpackagetool6 -t KWin/Script -r <name>   (remove, errors ignored)
        //   3. kpackagetool6 -t KWin/Script -i <zip>    (install via KPackage!)
        //   4. kwriteconfig6 kwinrc Plugins <name>Enabled true
        //   5. reconfigure + up to 6 polling rounds of (reconfigure + 5 s)
        // Direct fs writes alone do NOT register the package (this was the
        // original boot flakiness — kpackagetool is what Toshy relies on).
        let kp = kpackagetool_cmd();
        if kp.is_none() {
            anyhow::bail!("kpackagetool6/5 not found — cannot install KWin script");
        }
        let kp = kp.unwrap();
        let kw = kwriteconfig_cmd();

        let zip_path = std::env::temp_dir().join(format!("{SCRIPT_ID}.kwinscript"));
        {
            use std::io::Write as _;
            let file = std::fs::File::create(&zip_path)
                .with_context(|| format!("create {}", zip_path.display()))?;
            let mut zf = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zf.start_file("contents/code/main.js", opts)?;
            zf.write_all(SCRIPT_MAIN.as_bytes())?;
            zf.start_file("metadata.json", opts)?;
            zf.write_all(SCRIPT_NAME_META.as_bytes())?;
            zf.finish()?;
        }

        // remove old (ignore failure), then install
        let _ = std::process::Command::new(&kp)
            .args(["-t", "KWin/Script", "-r", SCRIPT_ID])
            .output();
        let out = std::process::Command::new(&kp)
            .args(["-t", "KWin/Script", "-i"])
            .arg(&zip_path)
            .output()?;
        if !out.status.success() {
            anyhow::bail!(
                "kpackagetool install failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let _ = std::fs::remove_file(&zip_path);
        tracing::info!("kwin adapter: script installed via {kp}.");

        // enable persistently
        let _ = std::process::Command::new(&kw)
            .args([
                "--file", "kwinrc", "--group", "Plugins", "--key",
                &format!("{SCRIPT_ID}Enabled"), "true",
            ])
            .output();

        // reconfigure + polling loop (6 rounds × 5 s, Toshy numbers)
        let _ = self.kwin_dbus_call("reconfigure", None);
        std::thread::sleep(std::time::Duration::from_secs(5));
        for _ in 0..6 {
            if self.script_loaded() {
                tracing::info!("kwin adapter: script installed & loaded ({}).", SCRIPT_ID);
                return Ok(());
            }
            let _ = self.kwin_dbus_call("reconfigure", None);
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        if self.script_loaded() {
            tracing::info!("kwin adapter: script installed & loaded ({}).", SCRIPT_ID);
            return Ok(());
        }
        Err(anyhow::anyhow!(
            "kwin script did not report loaded after 6 reconfigure rounds — background retry continues"
        ))
    }
    fn uninstall(&self) -> Result<()> {
        if let Some(kp) = kpackagetool_cmd() {
            let _ = std::process::Command::new(kp)
                .args(["-t", "KWin/Script", "-r", SCRIPT_ID])
                .output();
        }
        let _ = self.kwin_dbus_call("unloadScript", Some(SCRIPT_ID));
        let kw = kwriteconfig_cmd();
        {
            let _ = std::process::Command::new(kw)
                .args([
                    "--file", "kwinrc", "--group", "Plugins", "--key",
                    &format!("{SCRIPT_ID}Enabled"), "false",
                ])
                .output();
        }
        tracing::info!("kwin adapter: script removed");
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
