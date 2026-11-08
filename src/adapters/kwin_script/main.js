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
// initial sweep so the daemon has a context right after (re)load
workspace.windowList().forEach(notifyMackeys);
