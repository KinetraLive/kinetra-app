//! Keeps the displays awake while an output shows (owner's decision, macOS parity plan): a
//! display output that goes dark after the screen-saver timeout ends the show.
//! - Windows: a power request (`PowerCreateRequest` + `PowerRequestDisplayRequired`), held by
//!   handle, so it works from any thread and ends when the handle closes;
//! - macOS: an IOKit power assertion (`PreventUserIdleDisplaySleep`), released on drop.
//!
//! [`DisplayAwake`] holds it; dropping it lets the displays sleep again. Failing to get one is
//! logged and never stops an output.

/// While this lives, the displays don't go to sleep (and the system doesn't idle-sleep).
/// (Holds the request; only dropped.)
pub struct DisplayAwake(#[allow(dead_code)] sys::Hold);

impl DisplayAwake {
    /// `None` when the system refused (logged).
    pub fn hold(reason: &str) -> Option<Self> {
        match sys::Hold::new(reason) {
            Ok(h) => Some(Self(h)),
            Err(e) => {
                tracing::warn!("displays may sleep during the show: {e}");
                None
            }
        }
    }
}

#[cfg(windows)]
mod sys {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Power::{
        PowerClearRequest, PowerCreateRequest, PowerRequestDisplayRequired,
        PowerRequestSystemRequired, PowerSetRequest,
    };
    use windows::Win32::System::Threading::{
        POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
    };

    pub struct Hold(HANDLE);

    // SAFETY: a power request handle may be used and closed from any thread.
    unsafe impl Send for Hold {}

    impl Hold {
        pub fn new(reason: &str) -> Result<Self, String> {
            let mut text: Vec<u16> = reason.encode_utf16().chain(std::iter::once(0)).collect();
            let ctx = REASON_CONTEXT {
                Version: 0, // POWER_REQUEST_CONTEXT_VERSION
                Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
                Reason: REASON_CONTEXT_0 {
                    SimpleReasonString: PWSTR(text.as_mut_ptr()),
                },
            };
            unsafe {
                let h = PowerCreateRequest(&ctx).map_err(|e| e.to_string())?;
                let ok = PowerSetRequest(h, PowerRequestDisplayRequired)
                    .and_then(|()| PowerSetRequest(h, PowerRequestSystemRequired));
                if let Err(e) = ok {
                    let _ = CloseHandle(h);
                    return Err(e.to_string());
                }
                Ok(Self(h))
            }
        }
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            unsafe {
                let _ = PowerClearRequest(self.0, PowerRequestDisplayRequired);
                let _ = PowerClearRequest(self.0, PowerRequestSystemRequired);
                let _ = CloseHandle(self.0);
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod sys {
    use std::ffi::{c_char, c_void, CString};

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            kind: *const c_void,
            level: u32,
            name: *const c_void,
            id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            s: *const c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    const UTF8: u32 = 0x0800_0100;
    /// `kIOPMAssertionLevelOn`.
    const LEVEL_ON: u32 = 255;

    fn cf(s: &str) -> Result<*const c_void, String> {
        let c = CString::new(s).map_err(|e| e.to_string())?;
        let r = unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) };
        if r.is_null() {
            Err("no CFString".into())
        } else {
            Ok(r)
        }
    }

    pub struct Hold(u32);

    impl Hold {
        pub fn new(reason: &str) -> Result<Self, String> {
            // kIOPMAssertionTypePreventUserIdleDisplaySleep
            let kind = cf("PreventUserIdleDisplaySleep")?;
            let name = cf(reason)?;
            let mut id = 0u32;
            let r = unsafe { IOPMAssertionCreateWithName(kind, LEVEL_ON, name, &mut id) };
            unsafe {
                CFRelease(kind);
                CFRelease(name);
            }
            if r == 0 {
                Ok(Self(id))
            } else {
                Err(format!("IOPMAssertionCreateWithName: {r}"))
            }
        }
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            unsafe {
                IOPMAssertionRelease(self.0);
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod sys {
    pub struct Hold;

    impl Hold {
        pub fn new(_: &str) -> Result<Self, String> {
            Err("not supported on this system".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Taking and releasing the hold works on this system (Windows: a power request; macOS: an
    /// IOKit assertion; both harmless and gone when dropped).
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn holds_and_releases() {
        let h = DisplayAwake::hold("Kinetra test").expect("hold");
        drop(h);
        let again = DisplayAwake::hold("Kinetra test").expect("hold again");
        drop(again);
    }
}
