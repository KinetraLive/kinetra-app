//! This computer's identity for licensing: the fingerprint (`kinetra_license::machine`, a hash
//! of the OS machine id; only the hash leaves the machine) and its name (shown in the
//! account's device and computer lists).

/// The OS machine id: Windows `MachineGuid` (64-bit registry view). `None` when it can't be
/// read (then the app can't activate, but everything local keeps working).
pub fn os_machine_id() -> Option<String> {
    #[cfg(windows)]
    {
        use windows::core::w;
        use windows::Win32::System::Registry::{
            RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY,
        };
        let mut buf = [0u16; 128];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: the buffer and its byte length are valid for the call.
        let r = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                w!("SOFTWARE\\Microsoft\\Cryptography"),
                w!("MachineGuid"),
                RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
                None,
                Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
                Some(&mut len),
            )
        };
        if r.0 != 0 {
            return None;
        }
        let chars = (len as usize / 2).min(buf.len());
        let s = String::from_utf16_lossy(&buf[..chars]);
        let s = s.trim_end_matches('\0').trim().to_string();
        (!s.is_empty()).then_some(s)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// The fingerprint sent to the server and bound into tokens (64 hex).
pub fn fingerprint() -> Option<String> {
    os_machine_id().map(|id| kinetra_license::machine::fingerprint(&id))
}

/// The computer's name ("STUDIO-PC"), shown in the account's lists. Control characters
/// removed, at most 100 characters (API.md).
pub fn machine_name() -> String {
    let raw = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    let clean: String = raw.chars().filter(|c| !c.is_control()).take(100).collect();
    if clean.trim().is_empty() {
        "This computer".into()
    } else {
        clean.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(windows)]
    fn reads_this_machines_id() {
        let fp = super::fingerprint().expect("MachineGuid is readable");
        assert!(kinetra_license::machine::is_fingerprint(&fp));
        // Stable across calls.
        assert_eq!(Some(fp), super::fingerprint());
    }

    #[test]
    fn names_are_clean() {
        let n = super::machine_name();
        assert!(!n.is_empty() && n.len() <= 100 && !n.chars().any(char::is_control));
    }
}
