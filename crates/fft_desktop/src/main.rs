//! `fft_desktop` product binary.
//!
//! This is the composition root for the unified `fft_audio` application. It
//! parses the CLI and starts the GUI when a GUI feature is enabled, unless
//! `--headless` or a CLI adapter flag selects the CLI composition.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

/// NVIDIA's Wayland stack can abort GTK/WebKitGTK with
/// `Error 71 (Protocol error) dispatching to Wayland display`. Disabling
/// explicit sync avoids that crash.
#[cfg(target_os = "linux")]
fn linux_explicit_sync_value(current: Option<&str>) -> Option<&'static str> {
    if current.is_none() {
        Some("1")
    } else {
        None
    }
}

/// Set the NVIDIA/Wayland workaround unless the user already chose a value.
///
/// This runs before Tauri/GTK/WebKitGTK and before any worker threads start.
#[cfg(target_os = "linux")]
fn apply_linux_nvidia_wayland_workaround() {
    let current = std::env::var("__NV_DISABLE_EXPLICIT_SYNC").ok();
    if let Some(value) = linux_explicit_sync_value(current.as_deref()) {
        std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", value);
    }
}

/// Opt-in X11 fallback for NVIDIA/Wayland setups that still crash.
///
/// `FFT_AUDIO_FORCE_X11=1` sets GDK/Winit to X11 unless the user already chose
/// a backend explicitly.
#[cfg(target_os = "linux")]
fn apply_linux_x11_workaround() {
    if std::env::var_os("FFT_AUDIO_FORCE_X11").is_none() {
        return;
    }
    if std::env::var_os("GDK_BACKEND").is_none() {
        std::env::set_var("GDK_BACKEND", "x11");
    }
    if std::env::var_os("WINIT_UNIX_BACKEND").is_none() {
        std::env::set_var("WINIT_UNIX_BACKEND", "x11");
    }
}

fn main() -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        apply_linux_nvidia_wayland_workaround();
        apply_linux_x11_workaround();
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    match fft_desktop::parse_args(args).and_then(fft_desktop::run_product) {
        Ok(exit) => ExitCode::from(exit.exit_code() as u8),
        Err(error) if error.is_help() => {
            print!("{}", error.message());
            ExitCode::SUCCESS
        }
        Err(error) => {
            if error.is_rendered() {
                eprint!("{}", error.message());
            } else {
                eprintln!("fft_desktop: {error}");
            }
            ExitCode::from(error.exit_code() as u8)
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::linux_explicit_sync_value;

    #[test]
    fn linux_explicit_sync_defaults_only_when_unset() {
        assert_eq!(linux_explicit_sync_value(None), Some("1"));
        assert_eq!(linux_explicit_sync_value(Some("0")), None);
        assert_eq!(linux_explicit_sync_value(Some("1")), None);
        assert_eq!(linux_explicit_sync_value(Some("custom")), None);
    }
}
