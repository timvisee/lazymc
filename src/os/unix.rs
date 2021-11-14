/// Force kill process on Unix by sending SIGKILL.
///
/// This is unsafe because the PID isn't checked.
pub unsafe fn force_kill(pid: u32) -> bool {
    debug!(target: "lazymc", "Sending SIGKILL signal to {} to kill server", pid);
    let result = libc::kill(pid as i32, libc::SIGKILL);

    if result != 0 {
        trace!(target: "lazymc", "SIGKILL failed: {}", result);
    }

    result == 0
}

/// Gracefully kill process on Unix by sending SIGTERM.
///
/// This is unsafe because the PID isn't checked.
pub unsafe fn kill_gracefully(pid: u32) -> bool {
    debug!(target: "lazymc", "Sending SIGTERM signal to {} to kill server", pid);
    let result = libc::kill(pid as i32, libc::SIGTERM);

    if result != 0 {
        warn!(target: "lazymc", "Sending SIGTERM signal to server failed: {}", result);
    }

    result == 0
}
