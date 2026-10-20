//! This computer's identity for licensing: the fingerprint (`kinetra_license::machine`, a hash
//! of the OS machine id; only the hash leaves the machine) and its name (shown in the
//! account's device and computer lists).

/// The OS machine id: Windows `MachineGuid` (64-bit registry view), macOS `IOPlatformUUID`.
/// `None` when it can't be read (then the app can't activate, but everything local keeps
/// working). Both go through the same fingerprint hash.
pub fn os_machine_id() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        // `ioreg` is a process start: read once.
        static ID: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        ID.get_or_init(|| {
            let out = std::process::Command::new("/usr/sbin/ioreg")
                .args(["-rd1", "-c", "IOPlatformExpertDevice"])
                .output()
                .ok()?;
            if !out.status.success() {
                return None;
            }
            parse_ioreg_uuid(&String::from_utf8_lossy(&out.stdout))
        })
        .clone()
    }
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
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// The `IOPlatformUUID` value from `ioreg -rd1 -c IOPlatformExpertDevice` output, e.g.
/// `"IOPlatformUUID" = "4C4C4544-0035-3010-8052-B4C04F4E4D32"`.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn parse_ioreg_uuid(out: &str) -> Option<String> {
    out.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        if key.trim() != "\"IOPlatformUUID\"" {
            return None;
        }
        let v = value.trim().trim_matches('"').trim();
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// The fingerprint sent to the server and bound into tokens (64 hex).
pub fn fingerprint() -> Option<String> {
    let profile = crate::core::devprofile::active();
    os_machine_id().map(|id| {
        kinetra_license::machine::fingerprint(&crate::core::devprofile::machine_id(id, profile))
    })
}

/// The computer's name ("STUDIO-PC"), shown in the account's lists. Control characters
/// removed, at most 100 characters (API.md).
pub fn machine_name() -> String {
    crate::core::devprofile::machine_name(
        clean_name(&raw_name()),
        crate::core::devprofile::active(),
    )
}

#[cfg(target_os = "macos")]
fn raw_name() -> String {
    // The name from System Settings › General › Sharing; macOS sets no COMPUTERNAME and
    // apps don't inherit HOSTNAME. `scutil` is a process start: read once.
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        std::process::Command::new("/usr/sbin/scutil")
            .args(["--get", "ComputerName"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    })
    .clone()
}

#[cfg(not(target_os = "macos"))]
fn raw_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default()
}

fn clean_name(raw: &str) -> String {
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
        assert_eq!(super::clean_name("  \u{7}"), "This computer");
        assert_eq!(super::clean_name(" Studio Mac\n"), "Studio Mac");
    }

    #[test]
    fn reads_the_platform_uuid_from_ioreg() {
        let out = r#"+-o J316sAP  <class IOPlatformExpertDevice, id 0x100000254, registered, matched, active, busy 0 (5 ms), retain 39>
    {
      "IOPlatformSerialNumber" = "C02XXXXXXXXX"
      "IOPlatformUUID" = "4C4C4544-0035-3010-8052-B4C04F4E4D32"
      "IOConsoleSecurityInterest" = <"IOConsoleSecurityInterest">
    }"#;
        let id = super::parse_ioreg_uuid(out).expect("uuid");
        assert_eq!(id, "4C4C4544-0035-3010-8052-B4C04F4E4D32");
        // The same hash as Windows' MachineGuid: a valid 64-hex fingerprint.
        let fp = kinetra_license::machine::fingerprint(&id);
        assert!(kinetra_license::machine::is_fingerprint(&fp));
        assert_eq!(
            super::parse_ioreg_uuid("\"IOPlatformSerialNumber\" = \"x\""),
            None
        );
        assert_eq!(super::parse_ioreg_uuid("\"IOPlatformUUID\" = \"\""), None);
    }
}
