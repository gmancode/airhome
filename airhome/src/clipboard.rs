//! Put text on the Windows clipboard. egui's own `copy_text` only reaches the system clipboard when egui-winit is
//! built with its `clipboard` feature, which this app does not use, so call the Win32 API directly.

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

/// Clipboard format "Unicode text".
const CF_UNICODETEXT: u32 = 13;

/// Replace the clipboard with `text`. Returns false if Windows refused (e.g. another program holds the clipboard).
pub fn set_text(text: &str) -> bool {
    let wide: Vec<u16> = text.replace("\r\n", "\n").replace('\n', "\r\n").encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let result = (|| -> windows::core::Result<()> {
            EmptyClipboard()?;
            let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2)?;
            let ptr = GlobalLock(mem) as *mut u16;
            if ptr.is_null() {
                return Err(windows::core::Error::from_thread());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(mem);
            // From here Windows owns the memory.
            SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0)))?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result.is_ok()
    }
}

#[cfg(test)]
mod tests {
    /// Overwrites the real clipboard, so it only runs on request: `cargo test clipboard -- --ignored`.
    #[test]
    #[ignore]
    fn text_reaches_the_system_clipboard() {
        assert!(super::set_text("airhome clipboard test: привет\nline 2"));
    }
}
