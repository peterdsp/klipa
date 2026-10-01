//! Caller authentication for the privileged helper (macOS).
//!
//! A world-connectable socket is not a trust boundary on its own, so every
//! connection is validated by the connecting process's code signature, not
//! by its PID or a claimed name. We read the peer's kernel `audit_token`
//! (`getsockopt(LOCAL_PEERTOKEN)`), hand it to the Security framework to
//! obtain a `SecCode` for exactly that process, and check it against a
//! pinned designated requirement: the klipa bundle identifier, an Apple
//! anchor, and klipa's Team ID. A process that is not klipa, or not signed
//! by klipa's team, is rejected before any power operation runs.
//!
//! The expected Team ID is baked at build time (`KLIPA_TEAM_ID`, injected
//! from the signing identity in the release workflow). It is public
//! information (visible in any signed binary via `codesign`), not a
//! secret. When it is absent (unsigned local/dev build) the helper fails
//! closed and rejects every caller: an unsigned helper cannot be registered
//! via `SMAppService` anyway, so the app simply uses the admin-prompt path.
//! This keeps the production daemon strong with no dev-only backdoor.

use klipa_ipc::ErrorReason;

/// The klipa app bundle identifier the peer must present.
const APP_IDENTIFIER: &str = "dev.peterdsp.klipa";

/// The Team ID baked in at build time from the signing identity. Empty in
/// an unsigned build, which makes `requirement_string` return `None` and
/// the helper reject all callers.
const EXPECTED_TEAM_ID: Option<&str> = option_env!("KLIPA_TEAM_ID");

/// Build the designated requirement text a trusted caller must satisfy, or
/// `None` when no Team ID is known (fail closed). Pure, so the exact
/// requirement is unit-tested without the Security framework.
pub fn requirement_string(identifier: &str, team_id: Option<&str>) -> Option<String> {
    let team = team_id.map(str::trim).filter(|t| !t.is_empty())?;
    Some(format!(
        "identifier \"{identifier}\" and anchor apple generic and \
         certificate leaf[subject.OU] = \"{team}\""
    ))
}

/// Validate the peer on `fd`. `Ok(uid)` returns the peer's effective uid
/// (for multi-user ownership); `Err` names why it was rejected.
#[cfg(target_os = "macos")]
pub fn authenticate_peer(fd: std::os::unix::io::RawFd) -> Result<u32, ErrorReason> {
    let Some(req) = requirement_string(APP_IDENTIFIER, EXPECTED_TEAM_ID) else {
        return Err(ErrorReason::Unauthenticated);
    };
    macos::check(fd, &req)
}

/// Off macOS the helper is a no-op binary, so this is never reached in a
/// real daemon; keep a definition so the crate builds everywhere.
#[cfg(not(target_os = "macos"))]
pub fn authenticate_peer(_fd: std::os::unix::io::RawFd) -> Result<u32, ErrorReason> {
    Err(ErrorReason::Unauthenticated)
}

#[cfg(target_os = "macos")]
mod macos {
    use klipa_ipc::ErrorReason;
    use std::ffi::c_void;
    use std::os::unix::io::RawFd;

    // getsockopt level/name for the peer's audit token on a Unix socket.
    const SOL_LOCAL: i32 = 0;
    const LOCAL_PEERTOKEN: i32 = 0x006;
    const LOCAL_PEEREPID: i32 = 0x003;

    // Security framework.
    const K_SEC_CS_DEFAULT_FLAGS: u32 = 0;
    const ERR_SEC_SUCCESS: i32 = 0;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct AuditToken {
        val: [u32; 8],
    }

    extern "C" {
        fn getsockopt(
            socket: RawFd,
            level: i32,
            option_name: i32,
            option_value: *mut c_void,
            option_len: *mut u32,
        ) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithBytes(
            alloc: *const c_void,
            bytes: *const u8,
            num_bytes: isize,
            encoding: u32,
            is_external: u8,
        ) -> *const c_void;
        fn CFDataCreate(alloc: *const c_void, bytes: *const u8, length: isize) -> *const c_void;
        fn CFDictionaryCreate(
            alloc: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        static kSecGuestAttributeAudit: *const c_void;
        fn SecCodeCopyGuestWithAttributes(
            host: *const c_void,
            attributes: *const c_void,
            flags: u32,
            target: *mut *const c_void,
        ) -> i32;
        fn SecRequirementCreateWithString(
            text: *const c_void,
            flags: u32,
            requirement: *mut *const c_void,
        ) -> i32;
        fn SecCodeCheckValidity(
            code: *const c_void,
            flags: u32,
            requirement: *const c_void,
        ) -> i32;
    }

    const UTF8: u32 = 0x0800_0100;

    fn cfstr(s: &str) -> *const c_void {
        // SAFETY: valid pointer + length; UTF-8 is a supported encoding.
        unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0) }
    }

    /// The peer's effective uid via `LOCAL_PEEREPID`-adjacent peer creds.
    /// We take the uid from the audit token itself below; this is a
    /// fallback for diagnostics only.
    fn peer_epid(fd: RawFd) -> Option<u32> {
        let mut pid: u32 = 0;
        let mut len = std::mem::size_of::<u32>() as u32;
        // SAFETY: standard getsockopt read into a u32.
        let rc = unsafe {
            getsockopt(
                fd,
                SOL_LOCAL,
                LOCAL_PEEREPID,
                &mut pid as *mut u32 as *mut c_void,
                &mut len,
            )
        };
        (rc == 0).then_some(pid)
    }

    fn peer_audit_token(fd: RawFd) -> Option<AuditToken> {
        let mut token = AuditToken { val: [0u32; 8] };
        let mut len = std::mem::size_of::<AuditToken>() as u32;
        // SAFETY: `token` is sized exactly to the audit_token_t the kernel
        // writes; `len` is its byte length.
        let rc = unsafe {
            getsockopt(
                fd,
                SOL_LOCAL,
                LOCAL_PEERTOKEN,
                &mut token as *mut AuditToken as *mut c_void,
                &mut len,
            )
        };
        (rc == 0 && len as usize == std::mem::size_of::<AuditToken>()).then_some(token)
    }

    /// The uid recorded in the audit token (index 1 is the effective uid in
    /// the BSM audit token layout).
    fn token_uid(token: &AuditToken) -> u32 {
        token.val[1]
    }

    pub fn check(fd: RawFd, requirement: &str) -> Result<u32, ErrorReason> {
        let Some(token) = peer_audit_token(fd) else {
            return Err(ErrorReason::Unauthenticated);
        };
        let uid = token_uid(&token);
        // SAFETY: each created CF object is released on every path; the
        // Security calls follow their documented ownership (create -> use
        // -> release). A non-success OSStatus from any step is a rejection.
        unsafe {
            // Wrap the audit token bytes in a CFData.
            let bytes = std::slice::from_raw_parts(
                token.val.as_ptr() as *const u8,
                std::mem::size_of::<AuditToken>(),
            );
            let data = CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize);
            if data.is_null() {
                return Err(ErrorReason::Unauthenticated);
            }
            // attributes = { kSecGuestAttributeAudit: data }
            let keys = [kSecGuestAttributeAudit];
            let values = [data];
            let attrs = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const c_void,
            );
            if attrs.is_null() {
                CFRelease(data);
                return Err(ErrorReason::Unauthenticated);
            }
            let mut code: *const c_void = std::ptr::null();
            let rc = SecCodeCopyGuestWithAttributes(
                std::ptr::null(),
                attrs,
                K_SEC_CS_DEFAULT_FLAGS,
                &mut code,
            );
            CFRelease(attrs);
            CFRelease(data);
            if rc != ERR_SEC_SUCCESS || code.is_null() {
                return Err(ErrorReason::Unauthenticated);
            }

            let req_str = cfstr(requirement);
            if req_str.is_null() {
                CFRelease(code);
                return Err(ErrorReason::Unauthenticated);
            }
            let mut req: *const c_void = std::ptr::null();
            let rc = SecRequirementCreateWithString(req_str, K_SEC_CS_DEFAULT_FLAGS, &mut req);
            CFRelease(req_str);
            if rc != ERR_SEC_SUCCESS || req.is_null() {
                CFRelease(code);
                return Err(ErrorReason::Unauthenticated);
            }
            let valid = SecCodeCheckValidity(code, K_SEC_CS_DEFAULT_FLAGS, req);
            CFRelease(req);
            CFRelease(code);
            if valid != ERR_SEC_SUCCESS {
                let _ = peer_epid(fd); // touch for diagnostics parity
                return Err(ErrorReason::Unauthenticated);
            }
        }
        Ok(uid)
    }
}

#[cfg(test)]
mod tests {
    use super::requirement_string;

    #[test]
    fn requirement_pins_identifier_anchor_and_team() {
        let r = requirement_string("dev.peterdsp.klipa", Some("AB12CD34EF")).unwrap();
        assert!(r.contains("identifier \"dev.peterdsp.klipa\""));
        assert!(r.contains("anchor apple generic"));
        assert!(r.contains("certificate leaf[subject.OU] = \"AB12CD34EF\""));
    }

    #[test]
    fn no_team_id_fails_closed() {
        assert_eq!(requirement_string("dev.peterdsp.klipa", None), None);
        assert_eq!(requirement_string("dev.peterdsp.klipa", Some("   ")), None);
        assert_eq!(requirement_string("dev.peterdsp.klipa", Some("")), None);
    }
}
