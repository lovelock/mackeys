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

// Fresh report at script activation (KWin runs this on load AND on every
// reconfigure). The daemon's adapter install retry loop reconfigures until
// it succeeds, so after a daemon or session restart the current focus is
// reported exactly once, without needing timers (QJSEngine has none:
// calling setInterval here throws and kills the script — incident 2026-11-08).
if (typeof workspace.activeWindow !== "undefined") {
    notifyMackeys(workspace.activeWindow);
}
