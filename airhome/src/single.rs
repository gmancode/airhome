//! Single-instance guard (Windows). A second launch must not start a second copy that fights over the tray
//! icon, the log and the settings file; it asks the running copy to show its window instead.

use std::sync::atomic::{AtomicIsize, Ordering};

#[cfg(windows)]
mod ffi {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        pub fn CreateMutexW(attrs: *mut c_void, initial_owner: i32, name: *const u16) -> isize;
        pub fn CreateEventW(attrs: *mut c_void, manual_reset: i32, initial_state: i32, name: *const u16) -> isize;
        pub fn SetEvent(h: isize) -> i32;
        pub fn WaitForSingleObject(h: isize, ms: u32) -> u32;
        pub fn GetLastError() -> u32;
    }
}

/// Handle of the "show your window" event owned by the first instance (0 = none).
static SHOW_EVENT: AtomicIsize = AtomicIsize::new(0);

pub enum Role {
    /// We are the first (or only) instance.
    First,
    /// Another instance is already running; it has been notified (unless `quiet`).
    Second,
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `quiet`: don't ask the running instance to show its window (used for `--hidden` autostart launches).
pub fn acquire(quiet: bool) -> Role {
    #[cfg(windows)]
    unsafe {
        const ERROR_ALREADY_EXISTS: u32 = 183;
        let mutex = ffi::CreateMutexW(std::ptr::null_mut(), 1, wide(r"Local\airhome-single-instance").as_ptr());
        let already = ffi::GetLastError() == ERROR_ALREADY_EXISTS;
        // Auto-reset event; the same name opens the same event in both processes.
        let event = ffi::CreateEventW(std::ptr::null_mut(), 0, 0, wide(r"Local\airhome-show-window").as_ptr());
        if already {
            if !quiet && event != 0 {
                ffi::SetEvent(event);
            }
            return Role::Second;
        }
        // Keep both handles for the whole life of the process (the OS releases them on exit).
        let _ = mutex;
        SHOW_EVENT.store(event, Ordering::SeqCst);
    }
    #[cfg(not(windows))]
    let _ = quiet;
    Role::First
}

/// Calls `on_show` on a background thread every time another launch asks us to show the window.
pub fn watch(on_show: impl Fn() + Send + 'static) {
    #[cfg(windows)]
    {
        let ev = SHOW_EVENT.load(Ordering::SeqCst);
        if ev == 0 {
            return;
        }
        std::thread::Builder::new()
            .name("single-instance".into())
            .spawn(move || loop {
                const WAIT_OBJECT_0: u32 = 0;
                const INFINITE: u32 = 0xFFFF_FFFF;
                if unsafe { ffi::WaitForSingleObject(ev, INFINITE) } == WAIT_OBJECT_0 {
                    on_show();
                } else {
                    break;
                }
            })
            .ok();
    }
    #[cfg(not(windows))]
    let _ = on_show;
}
