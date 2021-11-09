#[cfg(unix)]
pub mod unix;

/// Gracefully kill process.
///
/// # Panics
///
/// Panics on platforms other than Unix.
#[allow(unreachable_code)]
pub fn kill_gracefully(pid: u32) {
    #[cfg(unix)]
    unsafe {
        unix::kill_gracefully(pid);
        return;
    }

    unimplemented!(
        "gracefully killing Minecraft server process not implemented on non-Unix platforms"
    );
}
