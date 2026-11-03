//! Windows toast notifications for problems that happen while the window is hidden in the tray.

use tauri_winrt_notification::{Duration, Toast};

/// The installer creates a Start-menu shortcut carrying this AppUserModelID; with it, notifications are
/// titled "airhome" and use our icon.
const OWN_APP_ID: &str = "airhome.app";

/// Which identity to notify under. Windows only shows toasts for an id that has a Start-menu shortcut, so
/// use our own id when the installer's shortcut exists and borrow PowerShell's otherwise (portable use).
fn app_id() -> &'static str {
    let installed = std::env::var_os("APPDATA")
        .map(|a| std::path::Path::new(&a).join(std::path::Path::new("Microsoft").join("Windows").join("Start Menu").join("Programs").join("airhome.lnk")).exists())
        .unwrap_or(false);
    if installed {
        OWN_APP_ID
    } else {
        Toast::POWERSHELL_APP_ID
    }
}

/// Show a toast and report whether Windows accepted it (used by the `--toast-test` dev switch).
pub fn toast_blocking(title: &str, body: &str) -> Result<(), String> {
    Toast::new(app_id())
        .title(title)
        .text1(body)
        .duration(Duration::Short)
        .show()
        .map_err(|e| e.to_string())
}

/// Show a toast. Runs on its own thread so a slow shell can never stall the UI.
pub fn toast(title: &str, body: &str) {
    let (title, body) = (title.to_owned(), body.to_owned());
    let _ = std::thread::Builder::new().name("toast".into()).spawn(move || {
        let res = Toast::new(app_id())
            .title(&title)
            .text1(&body)
            .duration(Duration::Short)
            .show();
        if let Err(e) = res {
            tracing::warn!("could not show notification: {e}");
        }
    });
}

/// Shorten a technical error so it fits a toast.
pub fn short(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= 140 {
        s.to_string()
    } else {
        let cut: String = s.chars().take(137).collect();
        format!("{cut}…")
    }
}
