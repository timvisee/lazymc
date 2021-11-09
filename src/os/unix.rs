/// Gracefully kill process on Unix by sending SIGTERM.
///
/// This is unsafe because the PID isn't checked.
pub unsafe fn kill_gracefully(pid: u32) {
    debug!(target: "lazymc", "Sending SIGTERM signal to {} to kill server", pid);
    let result = libc::kill(pid as i32, libc::SIGTERM);
    trace!(target: "lazymc", "SIGTERM result: {}", result);

    // TODO: send sigterm to childs as well?
    // TODO: handle error if result != 0
}
