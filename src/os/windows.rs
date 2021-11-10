use winapi::shared::minwindef::{FALSE, TRUE};
use winapi::shared::ntdef::NULL;
use winapi::um::handleapi::CloseHandle;
use winapi::um::processthreadsapi::{OpenProcess, TerminateProcess};
use winapi::um::winnt::PROCESS_TERMINATE;

/// Force kill process on Windows.
///
/// This is unsafe because the PID isn't checked.
pub unsafe fn force_kill(pid: u32) -> bool {
    debug!(target: "lazymc", "Sending force kill to {} to kill server", pid);
    let handle = OpenProcess(PROCESS_TERMINATE, FALSE, pid);
    if handle == NULL {
        warn!(target: "lazymc", "Failed to open process handle in order to kill it");
        return false;
    }

    let terminated = TerminateProcess(handle, 1) == TRUE;
    let closed = CloseHandle(handle) == TRUE;

    terminated && closed
}
