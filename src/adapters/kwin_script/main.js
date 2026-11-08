function notifyMackeys(window){
    // Guard: window may be null when the task switcher dialog has focus
    if (!window) { return; }
    var caption = window.caption || "UNDEF";
    var resourceClass = window.resourceClass || "UNDEF";
    var resourceName = window.resourceName || "UNDEF";
    callDBus(
        "org.mackeys.Daemon",
        "/org/mackeys/Daemon",
        "org.mackeys.Daemon",
        "NotifyActiveWindow",
        caption,
        resourceClass,
        resourceName
    );
}

workspace.windowActivated.connect(notifyMackeys);
// periodic re-report of the CURRENT focus (review I3): daemon restarts,
// config reloads and relogin all get a fresh context without kickstart.
//5s interval keeps stale-terminal contexts from lingering.
offerInterval = setInterval(function(){ notifyMackeys(workspace.activeWindow); }, 5000);
