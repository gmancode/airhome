//! Scheduling priorities so audio isn't starved when the PC is busy (games, builds).
//!
//! - process class HIGH while streaming (back to NORMAL afterwards),
//! - audio threads join the MMCSS "Pro Audio" task and run at TIME_CRITICAL priority.

use windows::core::w;
use windows::Win32::System::Threading::{
    AvSetMmThreadCharacteristicsW, GetCurrentProcess, GetCurrentThread, SetPriorityClass, SetThreadPriority,
    HIGH_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, THREAD_PRIORITY_TIME_CRITICAL,
};

/// Call from a thread that does real-time audio work (capture, packet pacing).
pub fn audio_thread() {
    unsafe {
        let mut task = 0u32;
        let _ = AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task);
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
    }
}

/// Raise (`true`) or restore (`false`) the whole process priority.
pub fn process_boost(on: bool) {
    unsafe {
        let class = if on { HIGH_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS };
        let _ = SetPriorityClass(GetCurrentProcess(), class);
    }
}
