//! Silence this PC's own speakers while streaming, so sound comes only from the HomePod.
//!
//! WASAPI loopback copies whatever Windows plays, but Windows keeps playing it locally too. We mute the
//! default output device ("endpoint") for the duration of the stream and put it back afterwards.
//!
//! Safety net: the previous state is written to `mute_state.json` before muting. If the program is killed
//! or crashes while the PC is muted, the next launch calls [`recover`] and puts the device back.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SavedState {
    /// Endpoint id (stable string) of the device we muted.
    device_id: String,
    /// Whether it was already muted before we touched it.
    was_muted: bool,
}

fn state_path() -> PathBuf {
    crate::settings::dir().join("mute_state.json")
}

fn init_com() {
    // Ignore the result: S_FALSE / RPC_E_CHANGED_MODE just mean COM is already set up on this thread.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn enumerator() -> Result<IMMDeviceEnumerator> {
    init_com();
    unsafe { Ok(CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?) }
}

fn endpoint_volume(device: &IMMDevice) -> Result<IAudioEndpointVolume> {
    unsafe { Ok(device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)?) }
}

fn device_id(device: &IMMDevice) -> Result<String> {
    unsafe {
        let id = device.GetId()?;
        let s = id.to_string().map_err(|e| anyhow!("bad device id: {e}"))?;
        windows::Win32::System::Com::CoTaskMemFree(Some(id.0 as *const _));
        Ok(s)
    }
}

fn open_by_id(id: &str) -> Result<IMMDevice> {
    let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { Ok(enumerator()?.GetDevice(PCWSTR(wide.as_ptr()))?) }
}

fn set_mute(id: &str, mute: bool) -> Result<()> {
    let vol = endpoint_volume(&open_by_id(id)?)?;
    unsafe { vol.SetMute(mute, std::ptr::null())? };
    Ok(())
}

/// Result of muting; call [`Muter::restore`] to undo. Dropping it also restores, so a cancelled
/// connection or an unwinding panic can't leave the PC silent.
pub struct Muter {
    state: SavedState,
    restored: bool,
}

/// Mute the current default output device. Returns `None` when there is nothing to undo
/// (the device was already muted by the user).
pub fn engage() -> Result<Option<Muter>> {
    let device = unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole)? };
    let id = device_id(&device)?;
    let vol = endpoint_volume(&device)?;
    let was_muted = unsafe { vol.GetMute()?.as_bool() };
    if was_muted {
        return Ok(None); // the user muted it themselves; leave it exactly as it is
    }

    let state = SavedState { device_id: id.clone(), was_muted };
    // Write the safety net BEFORE changing anything.
    if let Ok(text) = serde_json::to_string(&state) {
        let _ = std::fs::create_dir_all(crate::settings::dir());
        let _ = std::fs::write(state_path(), text);
    }
    unsafe { vol.SetMute(true, std::ptr::null())? };
    Ok(Some(Muter { state, restored: false }))
}

impl Muter {
    pub fn restore(mut self) {
        self.do_restore();
    }

    fn do_restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        if let Err(e) = set_mute(&self.state.device_id, self.state.was_muted) {
            tracing::error!("could not restore PC sound: {e}");
            return; // keep the state file so the next launch can retry
        }
        let _ = std::fs::remove_file(state_path());
    }
}

impl Drop for Muter {
    fn drop(&mut self) {
        self.do_restore();
    }
}

/// Call at startup: if a previous run died while the PC was muted, undo it.
pub fn recover() {
    // COM is initialised per thread, and this runs on the main thread BEFORE the window exists: the window
    // library needs single-threaded COM there, so touching COM here would crash the launch. Use a helper thread.
    let _ = std::thread::spawn(recover_inner).join();
}

fn recover_inner() {
    let Ok(text) = std::fs::read_to_string(state_path()) else { return };
    let Ok(state) = serde_json::from_str::<SavedState>(&text) else {
        let _ = std::fs::remove_file(state_path());
        return;
    };
    match set_mute(&state.device_id, state.was_muted) {
        Ok(()) => {
            tracing::warn!("restored PC sound left muted by a previous run");
            let _ = std::fs::remove_file(state_path());
        }
        Err(e) => {
            // Device may be gone (unplugged); nothing to restore then.
            tracing::warn!("could not restore PC sound state: {e}");
            let _ = std::fs::remove_file(state_path());
        }
    }
}
