#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

/// Force kill process.
///
/// Results in undefined behavior if PID is invalid.
#[allow(unreachable_code)]
pub fn force_kill(pid: u32) -> bool {
    #[cfg(unix)]
    unsafe {
        return unix::force_kill(pid);
    }

    #[cfg(windows)]
    unsafe {
        return windows::force_kill(pid);
    }

    unimplemented!("force killing Minecraft server process not implemented on this platform");
}

/// Gracefully kill process.
///
/// Results in undefined behavior if PID is invalid.
///
/// # Panics
///
/// Panics on platforms other than Unix.
#[allow(unreachable_code)]
pub fn kill_gracefully(pid: u32) -> bool {
    #[cfg(unix)]
    unsafe {
        return unix::kill_gracefully(pid);
    }

    unimplemented!(
        "gracefully killing Minecraft server process not implemented on non-Unix platforms"
    );
}
