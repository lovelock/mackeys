//! Kernel uevent watcher via NETLINK_KOBJECT_UEVENT (group 1).
//! Real kernel push events for input device add/remove — replaces the
//! polling-rescan hack (incident 2026-11-08: a periodic scan in an idle
//! branch silently failed to re-grab replugged keyboards).
//! Pure std + libc; every relevant event becomes `LoopMsg::Rescan`.

use anyhow::Result;
use crate::daemon::input::LoopMsg;

fn open_netlink_kobject_uevent() -> Result<std::os::unix::io::RawFd> {
    unsafe {
        let fd = libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            libc::NETLINK_KOBJECT_UEVENT as i32,
        );
        if fd < 0 {
            anyhow::bail!("netlink socket: {}", std::io::Error::last_os_error());
        }
        let mut addr: libc::sockaddr_nl = std::mem::zeroed();
        addr.nl_family = libc::AF_NETLINK as u16;
        addr.nl_groups = 1; // group 1 = unicast/broadcast kernel uevents

        let ret = libc::bind(
            fd,
            &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_nl>() as u32,
        );
        if ret != 0 {
            anyhow::bail!("netlink bind: {}", std::io::Error::last_os_error());
        }
        Ok(fd)
    }
}

/// Spawn the watcher thread: pushes `LoopMsg::Rescan` for every input
/// device add/remove uevent.多维块: the main loop's rescan_tick then opens
/// only what is missing — idempotent by construction.
pub fn spawn_watcher(
    tx: std::sync::mpsc::Sender<LoopMsg>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    let fd = open_netlink_kobject_uevent()?;
    std::thread::Builder::new()
        .name("mackeys-udev-watch".into())
        .spawn(move || {
            let mut buf = vec![0u8; 65536];
            loop {
                if stop.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let n = unsafe {
                    libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0)
                };
                if n <= 0 {
                    // netlink socket broken — exit thread; the fallback
                    // periodic rescan in the main loop keeps working
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    continue;
                }
                let payload = String::from_utf8_lossy(&buf[..n as usize]);
                let first = payload.lines().next().unwrap_or("");
                // "add@/devices/.../input/event42" / "remove@..."
                let touches_input = first.contains("/input/input") && first.contains("event");
                if touches_input {
                    tracing::debug!(asset = %first, "uevent");
                    if tx.send(LoopMsg::Rescan).is_err() {
                        break;
                    }
                }
            }
        })?;
    Ok(())
}
