//! Secrets (the calendar's private iCal link) live in Windows Credential Manager, encrypted per user,
//! never in settings.json or logs.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn target(name: &str) -> Vec<u16> {
    wide(&format!("Nabra/{name}"))
}

pub fn set(name: &str, value: &str) -> Result<(), String> {
    let mut t = target(name);
    let mut user = wide("nabra");
    let blob = value.as_bytes();
    let cred = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(t.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_ptr() as *mut u8,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(user.as_mut_ptr()),
        ..Default::default()
    };
    unsafe { CredWriteW(&cred, 0) }.map_err(|e| format!("Couldn't save to Credential Manager: {e}"))
}

pub fn get(name: &str) -> Option<String> {
    let t = target(name);
    let mut found: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe {
        CredReadW(PCWSTR(t.as_ptr()), CRED_TYPE_GENERIC, None, &mut found).ok()?;
        let c = &*found;
        let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec();
        CredFree(found as *const _);
        String::from_utf8(bytes).ok()
    }
}

pub fn delete(name: &str) {
    let t = target(name);
    unsafe {
        let _ = CredDeleteW(PCWSTR(t.as_ptr()), CRED_TYPE_GENERIC, None);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let name = format!("selftest-{}", std::process::id());
        super::set(&name, "https://example.com/secret.ics").unwrap();
        assert_eq!(super::get(&name).as_deref(), Some("https://example.com/secret.ics"));
        super::delete(&name);
        assert_eq!(super::get(&name), None);
    }
}
