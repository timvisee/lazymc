use winapi::um::handleapi::CloseHandle;
use winapi::um::processthreadsapi::{OpenProcess, TerminateProcess};
use winapi::um::winnt::PROCESS_TERMINATE;

/// Force kill process on Windows.
///
/// This is unsafe because the PID isn't checked.
pub unsafe fn force_kill(pid: u32) -> bool {
    debug!(target: "lazymc", "Sending force kill to {} to kill server", pid);
    let handle = OpenProcess(PROCESS_TERMINATE, false, pid);
    let mut ok = TerminateProcess(handle, 1);
    CloseHandle(handle) && ok
}
