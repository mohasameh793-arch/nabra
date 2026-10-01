//! "Start with Windows": the same HKCU Run entry the installer writes (desktop/installer-hooks.nsh).

use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: PCWSTR = w!("Nabra");

/// The command Windows runs at sign-in, if Nabra is registered.
pub fn command() -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, NAME, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
            .ok()
            .ok()?;
    }
    let len = (size as usize / 2).saturating_sub(1); // size includes the terminating NUL
    Some(String::from_utf16_lossy(&buf[..len]))
}

pub fn enabled() -> bool {
    command().is_some()
}

/// Registers (or removes) this executable to start at sign-in.
pub fn set(on: bool) -> Result<(), String> {
    if !on {
        unsafe {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, NAME);
        }
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    write(&format!("\"{}\"", exe.display()))
}

fn write(command: &str) -> Result<(), String> {
    let value: Vec<u16> = command.encode_utf16().chain([0]).collect();
    unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, NAME, REG_SZ.0, Some(value.as_ptr().cast()), (value.len() * 2) as u32)
            .ok()
            .map_err(|e| format!("Couldn't turn on start with Windows: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_and_restores_the_users_setting() {
        let before = command();
        set(true).unwrap();
        let cmd = command().expect("registered");
        assert!(cmd.starts_with('"') && cmd.contains(".exe"), "{cmd}");
        set(false).unwrap();
        assert!(!enabled());
        if let Some(original) = before {
            write(&original).unwrap(); // put back whatever the user had
        }
    }
}
