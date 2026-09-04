//! Sandbox-safe clamshell (lid-closed) readiness detection for macOS.
//!
//! This is the App-Store-compliant companion to the lid-closed *keep
//! awake* feature. Overriding lid-close sleep needs root and only ships
//! in the direct build (see `docs/lid-closed-keep-awake.md`). This module
//! instead only *reads* the display and power state via public
//! CoreGraphics / IOKit calls, so it works inside the App Sandbox and
//! ships in every macOS build, including the Mac App Store one.
//!
//! It answers one honest question for the menu: if you close the lid right
//! now, will the Mac keep running or go to sleep? A MacBook runs with the
//! lid closed only in Apple's supported clamshell mode: an external
//! display attached, plus AC power on Intel (Apple Silicon can run
//! clamshell on battery). No public API lets a sandboxed app override the
//! lid switch itself, so the honest thing is to detect and report.

/// What happens to this Mac if the lid closes right now.
///
/// Off macOS only `Hidden` is ever produced, so the other variants would
/// read as "never constructed" there; they are real everywhere it matters.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum ClamshellStatus {
    /// Closing the lid keeps the Mac running: an external display is
    /// attached, and the hardware's power requirement is met.
    StaysAwake,
    /// External display attached, but this Intel Mac needs AC power first.
    NeedsPower,
    /// No external display: closing the lid will sleep the Mac.
    WillSleep,
    /// Nothing to show: a desktop Mac (no built-in display), a non-macOS
    /// platform, or detection was unavailable.
    Hidden,
}

#[cfg(target_os = "macos")]
pub fn status() -> ClamshellStatus {
    macos::status()
}

#[cfg(not(target_os = "macos"))]
pub fn status() -> ClamshellStatus {
    // Clamshell is a macOS laptop concept; nothing to report elsewhere.
    ClamshellStatus::Hidden
}

#[cfg(target_os = "macos")]
mod macos {
    use super::ClamshellStatus;
    use std::ffi::{c_void, CStr};
    use std::os::raw::c_char;

    type CGDirectDisplayID = u32;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGGetActiveDisplayList(
            max_displays: u32,
            active_displays: *mut CGDirectDisplayID,
            display_count: *mut u32,
        ) -> i32;
        fn CGDisplayIsBuiltin(display: CGDirectDisplayID) -> i32;
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        // Owned (Copy) blob of the current power sources; must CFRelease.
        fn IOPSCopyPowerSourcesInfo() -> *const c_void;
        // Unowned CFString: "AC Power" / "Battery Power" / "UPS Power".
        fn IOPSGetProvidingPowerSourceType(snapshot: *const c_void) -> *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetCString(
            the_string: *const c_void,
            buffer: *mut c_char,
            buffer_size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(cf: *const c_void);
    }

    // libSystem is always linked on macOS; no #[link] needed.
    extern "C" {
        fn sysctlbyname(
            name: *const c_char,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> i32;
    }

    // kCFStringEncodingUTF8.
    const UTF8: u32 = 0x0800_0100;
    // kCGErrorSuccess.
    const CG_SUCCESS: i32 = 0;

    pub fn status() -> ClamshellStatus {
        let (has_builtin, has_external) = displays();
        if !has_builtin {
            // No internal display => desktop Mac (or detection failed):
            // there is no lid to reason about.
            return ClamshellStatus::Hidden;
        }
        if !has_external {
            return ClamshellStatus::WillSleep;
        }
        // An external display is attached, so clamshell is possible. Apple
        // Silicon runs clamshell on battery; Intel requires AC power.
        if is_apple_silicon() || power_is_ac() {
            ClamshellStatus::StaysAwake
        } else {
            ClamshellStatus::NeedsPower
        }
    }

    /// `(has_builtin, has_external)` across the active displays.
    fn displays() -> (bool, bool) {
        // First call: ask only for the count.
        let mut count: u32 = 0;
        // SAFETY: null buffer with a valid out-count is the documented way
        // to query the number of active displays.
        let rc = unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) };
        if rc != CG_SUCCESS || count == 0 {
            return (false, false);
        }
        let mut ids = vec![0 as CGDirectDisplayID; count as usize];
        // SAFETY: `ids` is sized to `count`; the call fills up to `count`.
        let rc = unsafe { CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut count) };
        if rc != CG_SUCCESS {
            return (false, false);
        }
        let mut builtin = false;
        let mut external = false;
        for &id in ids.iter().take(count as usize) {
            // SAFETY: `id` came from the active-display list above.
            if unsafe { CGDisplayIsBuiltin(id) } != 0 {
                builtin = true;
            } else {
                external = true;
            }
        }
        (builtin, external)
    }

    fn power_is_ac() -> bool {
        // SAFETY: IOPSCopyPowerSourcesInfo hands back an owned blob we must
        // release; IOPSGetProvidingPowerSourceType returns an unowned
        // CFString into that blob, valid until we release the blob.
        unsafe {
            let snapshot = IOPSCopyPowerSourcesInfo();
            if snapshot.is_null() {
                return false;
            }
            let src = IOPSGetProvidingPowerSourceType(snapshot);
            let mut buf = [0 as c_char; 64];
            let ok = if src.is_null() {
                0
            } else {
                CFStringGetCString(src, buf.as_mut_ptr(), buf.len() as isize, UTF8)
            };
            CFRelease(snapshot);
            if ok == 0 {
                return false;
            }
            CStr::from_ptr(buf.as_ptr()).to_str().unwrap_or("") == "AC Power"
        }
    }

    /// True on Apple Silicon, including an x86_64 binary under Rosetta,
    /// because `hw.optional.arm64` reflects the real hardware.
    fn is_apple_silicon() -> bool {
        let name = b"hw.optional.arm64\0";
        let mut result: i32 = 0;
        let mut size = std::mem::size_of::<i32>();
        // SAFETY: standard sysctl read into a correctly-sized i32 out-param.
        let rc = unsafe {
            sysctlbyname(
                name.as_ptr() as *const c_char,
                &mut result as *mut i32 as *mut c_void,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        rc == 0 && result == 1
    }
}
