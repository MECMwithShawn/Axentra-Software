//! Windows WebAuthn (webauthn.dll) passkey PRF: register a passkey with PRF enabled and evaluate
//! PRF on it. Struct layouts follow webauthn.h, declared only up to the versions requested; this
//! is the same interop the KeePass plugin uses (`keepass-plugin/src/WebAuthn.cs`).
#![cfg(windows)]

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use zeroize::Zeroizing;

pub type Hwnd = *mut c_void;

const UV_REQUIRED: u32 = 1;
const ATTESTATION_NONE: u32 = 1;
const TIMEOUT_MS: u32 = 120_000;
const COSE_ES256: i32 = -7;
const COSE_RS256: i32 = -257;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct PasskeyError {
    /// WebAuthn error name, e.g. `NotAllowedError` when the user cancels.
    pub name: String,
    pub message: String,
}

impl PasskeyError {
    fn new(name: &str, message: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            message: message.into(),
        }
    }

    /// The user dismissed the dialog or it timed out.
    pub fn is_cancellation(&self) -> bool {
        self.name == "NotAllowedError" || self.name == "AbortError"
    }
}

pub struct Registration {
    pub credential_id: Vec<u8>,
    pub prf_enabled: bool,
    /// WEBAUTHN_CTAP_TRANSPORT_* flag of the authenticator used (0 when Windows does not say).
    pub transport: u32,
}

pub const TRANSPORT_USB: u32 = 0x1;
pub const TRANSPORT_NFC: u32 = 0x2;
pub const TRANSPORT_BLE: u32 = 0x4;
pub const TRANSPORT_INTERNAL: u32 = 0x10;
pub const TRANSPORT_HYBRID: u32 = 0x20;

/// A short name for the kind of authenticator, e.g. "Security key (USB)" for a YubiKey.
pub fn transport_label(transport: u32) -> &'static str {
    if transport & TRANSPORT_USB != 0 {
        "Security key (USB)"
    } else if transport & TRANSPORT_NFC != 0 {
        "Security key (NFC)"
    } else if transport & TRANSPORT_HYBRID != 0 {
        "Phone passkey"
    } else if transport & TRANSPORT_BLE != 0 {
        "Bluetooth key"
    } else if transport & TRANSPORT_INTERNAL != 0 {
        "Windows Hello"
    } else {
        "Passkey"
    }
}

/// A registered passkey and the PRF input its envelope was sealed with.
pub struct PrfCredential<'a> {
    pub credential_id: &'a [u8],
    pub prf_input: &'a [u8; 32],
}

#[repr(C)]
struct ClientData {
    version: u32,
    cb_json: u32,
    pb_json: *const u8,
    hash_alg: *const u16,
}
#[repr(C)]
struct RpEntity {
    version: u32,
    id: *const u16,
    name: *const u16,
    icon: *const u16,
}
#[repr(C)]
struct UserEntity {
    version: u32,
    cb_id: u32,
    pb_id: *const u8,
    name: *const u16,
    icon: *const u16,
    display_name: *const u16,
}
#[repr(C)]
struct CoseParam {
    version: u32,
    cred_type: *const u16,
    alg: i32,
}
#[repr(C)]
struct CoseParams {
    count: u32,
    params: *const CoseParam,
}
#[repr(C)]
struct Credential {
    version: u32,
    cb_id: u32,
    pb_id: *const u8,
    cred_type: *const u16,
}
#[repr(C)]
struct Credentials {
    count: u32,
    creds: *const Credential,
}
#[repr(C)]
struct Extensions {
    count: u32,
    exts: *const c_void,
}
/// WEBAUTHN_EXTENSION.
#[repr(C)]
struct Extension {
    id: *const u16,
    cb: u32,
    pv: *const c_void,
}
#[repr(C)]
struct HmacSecretSalt {
    cb_first: u32,
    pb_first: *const u8,
    cb_second: u32,
    pb_second: *const u8,
}
/// WEBAUTHN_CRED_WITH_HMAC_SECRET_SALT: a PRF input for one credential.
#[repr(C)]
struct CredWithSalt {
    cb_cred_id: u32,
    pb_cred_id: *const u8,
    salt: *const HmacSecretSalt,
}
#[repr(C)]
struct HmacSecretSaltValues {
    global: *const HmacSecretSalt,
    cred_count: u32,
    cred_salts: *const CredWithSalt,
}

/// WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS through version 6.
#[repr(C)]
struct MakeCredentialOptions {
    version: u32,
    timeout_ms: u32,
    credential_list: Credentials,
    extensions: Extensions,
    attachment: u32,
    require_resident_key: i32,
    user_verification: u32,
    attestation: u32,
    flags: u32,
    cancellation_id: *const c_void, // v2
    exclude_list: *const c_void,    // v3
    enterprise_attestation: u32,    // v4
    large_blob_support: u32,
    prefer_resident_key: i32,
    browser_in_private_mode: i32, // v5
    enable_prf: i32,              // v6
}

/// WEBAUTHN_CREDENTIAL_ATTESTATION through version 5.
#[repr(C)]
struct CredentialAttestation {
    version: u32,
    format_type: *const u16,
    cb_authenticator_data: u32,
    pb_authenticator_data: *const u8,
    cb_attestation: u32,
    pb_attestation: *const u8,
    attestation_decode_type: u32,
    attestation_decode: *const c_void,
    cb_attestation_object: u32,
    pb_attestation_object: *const u8,
    cb_credential_id: u32,
    pb_credential_id: *const u8,
    extensions: Extensions, // v2
    used_transport: u32,    // v3
    ep_att: i32,            // v4
    large_blob_supported: i32,
    resident_key: i32,
    prf_enabled: i32, // v5
}

/// WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS through version 6.
#[repr(C)]
struct GetAssertionOptions {
    version: u32,
    timeout_ms: u32,
    credential_list: Credentials,
    extensions: Extensions,
    attachment: u32,
    user_verification: u32,
    flags: u32,
    u2f_app_id: *const u16, // v2
    u2f_app_id_used: *mut i32,
    cancellation_id: *const c_void, // v3
    allow_list: *const c_void,      // v4
    cred_large_blob_operation: u32, // v5
    cb_cred_large_blob: u32,
    pb_cred_large_blob: *const u8,
    hmac_secret_salt_values: *const HmacSecretSaltValues, // v6
    browser_in_private_mode: i32,
}

/// WEBAUTHN_ASSERTION through version 4.
#[repr(C)]
struct Assertion {
    version: u32,
    cb_authenticator_data: u32,
    pb_authenticator_data: *const u8,
    cb_signature: u32,
    pb_signature: *const u8,
    credential: Credential,
    cb_user_id: u32,
    pb_user_id: *const u8,
    extensions: Extensions,  // v2
    cb_cred_large_blob: u32, // v3
    pb_cred_large_blob: *const u8,
    cred_large_blob_status: u32,
    hmac_secret: *const HmacSecretSalt, // v4
}

#[link(name = "webauthn", kind = "raw-dylib")]
unsafe extern "system" {
    fn WebAuthNGetApiVersionNumber() -> u32;
    fn WebAuthNAuthenticatorMakeCredential(
        hwnd: Hwnd,
        rp: *const RpEntity,
        user: *const UserEntity,
        params: *const CoseParams,
        client_data: *const ClientData,
        options: *const MakeCredentialOptions,
        attestation: *mut *mut CredentialAttestation,
    ) -> i32;
    fn WebAuthNAuthenticatorGetAssertion(
        hwnd: Hwnd,
        rp_id: *const u16,
        client_data: *const ClientData,
        options: *const GetAssertionOptions,
        assertion: *mut *mut Assertion,
    ) -> i32;
    fn WebAuthNFreeCredentialAttestation(attestation: *mut CredentialAttestation);
    fn WebAuthNFreeAssertion(assertion: *mut Assertion);
    fn WebAuthNGetErrorName(hr: i32) -> *const u16;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetForegroundWindow() -> Hwnd;
}

/// The window the passkey dialog should be parented to (the app's, while it is focused).
pub fn foreground_window() -> Hwnd {
    unsafe { GetForegroundWindow() }
}

pub fn api_version() -> u32 {
    unsafe { WebAuthNGetApiVersionNumber() }
}

fn require_api() -> Result<(), PasskeyError> {
    let v = api_version();
    if v < 6 {
        return Err(PasskeyError::new(
            "NotSupportedError",
            format!(
                "This version of Windows does not support passkey PRF (WebAuthn API {v}, need 6)."
            ),
        ));
    }
    Ok(())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn check(hr: i32, what: &str) -> Result<(), PasskeyError> {
    if hr == 0 {
        return Ok(());
    }
    let name = unsafe {
        let p = WebAuthNGetErrorName(hr);
        if p.is_null() {
            "UnknownError".to_owned()
        } else {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
        }
    };
    Err(PasskeyError::new(
        &name,
        format!("{what} failed: {name} (0x{:08X})", hr as u32),
    ))
}

fn client_data_json(kind: &str, rp_id: &str) -> Vec<u8> {
    // There is no relying-party server: the challenge only needs to be fresh.
    let mut challenge = [0u8; 32];
    getrandom::fill(&mut challenge).expect("OS RNG");
    let b64 = base64url(&challenge);
    format!("{{\"type\":\"{kind}\",\"challenge\":\"{b64}\",\"origin\":\"https://{rp_id}\",\"crossOrigin\":false}}").into_bytes()
}

fn base64url(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// Registers a discoverable passkey with PRF enabled. Blocks until the user finishes (up to 2 min).
// `hwnd` is only handed to Windows as the dialog's parent window, never dereferenced here.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn register(
    hwnd: Hwnd,
    rp_id: &str,
    rp_name: &str,
    user_id: &[u8],
    user_name: &str,
    exclude: &[&[u8]],
) -> Result<Registration, PasskeyError> {
    require_api()?;
    let (w_rp_id, w_rp_name, w_user, w_type, w_sha) = (
        wide(rp_id),
        wide(rp_name),
        wide(user_name),
        wide("public-key"),
        wide("SHA-256"),
    );
    let json = client_data_json("webauthn.create", rp_id);
    // CTAP security keys (YubiKey) only create the per-credential PRF secret when `hmac-secret`
    // is requested at registration; Windows' bEnablePrf alone covers phones and Windows Hello.
    let w_hmac = wide("hmac-secret");
    let hmac_true: i32 = 1;
    let hmac_ext = Extension {
        id: w_hmac.as_ptr(),
        cb: std::mem::size_of::<i32>() as u32,
        pv: (&hmac_true as *const i32).cast(),
    };
    // Keys already registered for this vault: the authenticator refuses to make a second
    // credential instead of silently adding a duplicate.
    let excluded: Vec<Credential> = exclude
        .iter()
        .map(|id| Credential {
            version: 1,
            cb_id: id.len() as u32,
            pb_id: id.as_ptr(),
            cred_type: w_type.as_ptr(),
        })
        .collect();

    let rp = RpEntity {
        version: 1,
        id: w_rp_id.as_ptr(),
        name: w_rp_name.as_ptr(),
        icon: null(),
    };
    let user = UserEntity {
        version: 1,
        cb_id: user_id.len() as u32,
        pb_id: user_id.as_ptr(),
        name: w_user.as_ptr(),
        icon: null(),
        display_name: w_user.as_ptr(),
    };
    let params = [
        CoseParam {
            version: 1,
            cred_type: w_type.as_ptr(),
            alg: COSE_ES256,
        },
        CoseParam {
            version: 1,
            cred_type: w_type.as_ptr(),
            alg: COSE_RS256,
        },
    ];
    let cose = CoseParams {
        count: params.len() as u32,
        params: params.as_ptr(),
    };
    let cd = ClientData {
        version: 1,
        cb_json: json.len() as u32,
        pb_json: json.as_ptr(),
        hash_alg: w_sha.as_ptr(),
    };
    let opts = MakeCredentialOptions {
        version: 6,
        timeout_ms: TIMEOUT_MS,
        credential_list: Credentials {
            count: excluded.len() as u32,
            creds: if excluded.is_empty() {
                null()
            } else {
                excluded.as_ptr()
            },
        },
        extensions: Extensions {
            count: 1,
            exts: (&hmac_ext as *const Extension).cast(),
        },
        attachment: 0,
        require_resident_key: 1,
        user_verification: UV_REQUIRED,
        attestation: ATTESTATION_NONE,
        flags: 0,
        cancellation_id: null(),
        exclude_list: null(),
        enterprise_attestation: 0,
        large_blob_support: 0,
        prefer_resident_key: 1,
        browser_in_private_mode: 0,
        enable_prf: 1,
    };

    let mut att: *mut CredentialAttestation = null_mut();
    check(
        unsafe {
            WebAuthNAuthenticatorMakeCredential(hwnd, &rp, &user, &cose, &cd, &opts, &mut att)
        },
        "Passkey registration",
    )?;
    unsafe {
        let a = &*att;
        let reg = Registration {
            credential_id: std::slice::from_raw_parts(
                a.pb_credential_id,
                a.cb_credential_id as usize,
            )
            .to_vec(),
            // Older attestation structs do not report PRF; the first evaluation is the real test.
            prf_enabled: a.version < 5 || a.prf_enabled != 0 || hmac_secret_created(a),
            transport: if a.version >= 3 { a.used_transport } else { 0 },
        };
        WebAuthNFreeCredentialAttestation(att);
        Ok(reg)
    }
}

/// True when the attestation's extension outputs include `hmac-secret: true`.
unsafe fn hmac_secret_created(a: &CredentialAttestation) -> bool {
    if a.version < 2 || a.extensions.count == 0 || a.extensions.exts.is_null() {
        return false;
    }
    let exts = unsafe {
        std::slice::from_raw_parts(
            a.extensions.exts as *const Extension,
            a.extensions.count as usize,
        )
    };
    exts.iter().any(|e| {
        !e.id.is_null()
            && unsafe { wide_eq(e.id, "hmac-secret") }
            && e.cb as usize >= std::mem::size_of::<i32>()
            && !e.pv.is_null()
            && unsafe { *(e.pv as *const i32) } != 0
    })
}

unsafe fn wide_eq(p: *const u16, s: &str) -> bool {
    let len = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) }) == s
}

/// Evaluates PRF(`prf_input`) on the given passkey. Windows applies the WebAuthn PRF salt hashing,
/// so the output equals a browser's `prf.eval.first`. Blocks until the user finishes.
pub fn evaluate_prf(
    hwnd: Hwnd,
    rp_id: &str,
    credential_id: &[u8],
    prf_input: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>, PasskeyError> {
    let cred = PrfCredential {
        credential_id,
        prf_input,
    };
    evaluate_prf_any(hwnd, rp_id, &[cred]).map(|(_, out)| out)
}

/// Offers every given passkey in one Windows dialog (phone, security key, ...) and evaluates PRF
/// on whichever the user picks. Returns the index of the credential used and its PRF output.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn evaluate_prf_any(
    hwnd: Hwnd,
    rp_id: &str,
    creds: &[PrfCredential],
) -> Result<(usize, Zeroizing<[u8; 32]>), PasskeyError> {
    require_api()?;
    if creds.is_empty() {
        return Err(PasskeyError::new("NotAllowedError", "No passkeys to ask."));
    }
    let (w_rp_id, w_type, w_sha) = (wide(rp_id), wide("public-key"), wide("SHA-256"));
    let json = client_data_json("webauthn.get", rp_id);

    let allowed: Vec<Credential> = creds
        .iter()
        .map(|c| Credential {
            version: 1,
            cb_id: c.credential_id.len() as u32,
            pb_id: c.credential_id.as_ptr(),
            cred_type: w_type.as_ptr(),
        })
        .collect();
    let salt_of = |input: &[u8; 32]| HmacSecretSalt {
        cb_first: 32,
        pb_first: input.as_ptr(),
        cb_second: 0,
        pb_second: null(),
    };
    // One shared PRF input goes in the global salt (the path proven with an iPhone); only rings
    // whose keys were sealed with different inputs need per-credential salts.
    let shared = creds.iter().all(|c| c.prf_input == creds[0].prf_input);
    let global = salt_of(creds[0].prf_input);
    let per_cred: Vec<HmacSecretSalt> = creds.iter().map(|c| salt_of(c.prf_input)).collect();
    let with_salts: Vec<CredWithSalt> = creds
        .iter()
        .zip(&per_cred)
        .map(|(c, salt)| CredWithSalt {
            cb_cred_id: c.credential_id.len() as u32,
            pb_cred_id: c.credential_id.as_ptr(),
            salt,
        })
        .collect();
    let salts = if shared {
        HmacSecretSaltValues {
            global: &global,
            cred_count: 0,
            cred_salts: null(),
        }
    } else {
        HmacSecretSaltValues {
            global: null(),
            cred_count: with_salts.len() as u32,
            cred_salts: with_salts.as_ptr(),
        }
    };
    let cd = ClientData {
        version: 1,
        cb_json: json.len() as u32,
        pb_json: json.as_ptr(),
        hash_alg: w_sha.as_ptr(),
    };
    let opts = GetAssertionOptions {
        version: 6,
        timeout_ms: TIMEOUT_MS,
        credential_list: Credentials {
            count: allowed.len() as u32,
            creds: allowed.as_ptr(),
        },
        extensions: Extensions {
            count: 0,
            exts: null(),
        },
        attachment: 0,
        user_verification: UV_REQUIRED,
        flags: 0,
        u2f_app_id: null(),
        u2f_app_id_used: null_mut(),
        cancellation_id: null(),
        allow_list: null(),
        cred_large_blob_operation: 0,
        cb_cred_large_blob: 0,
        pb_cred_large_blob: null(),
        hmac_secret_salt_values: &salts,
        browser_in_private_mode: 0,
    };

    let mut assertion: *mut Assertion = null_mut();
    check(
        unsafe {
            WebAuthNAuthenticatorGetAssertion(hwnd, w_rp_id.as_ptr(), &cd, &opts, &mut assertion)
        },
        "Passkey unlock",
    )?;
    unsafe {
        let a = &*assertion;
        let index = if creds.len() == 1 {
            Some(0)
        } else {
            let used = std::slice::from_raw_parts(a.credential.pb_id, a.credential.cb_id as usize);
            creds.iter().position(|c| c.credential_id == used)
        };
        let result = if a.version < 4 || a.hmac_secret.is_null() {
            Err(PasskeyError::new(
                "NotSupportedError",
                "The passkey did not return a PRF secret.",
            ))
        } else if let Some(index) = index {
            let s = &*a.hmac_secret;
            if s.cb_first != 32 {
                Err(PasskeyError::new(
                    "NotSupportedError",
                    format!("Unexpected PRF output length {}.", s.cb_first),
                ))
            } else {
                let mut out = Zeroizing::new([0u8; 32]);
                out.copy_from_slice(std::slice::from_raw_parts(s.pb_first, 32));
                Ok((index, out))
            }
        } else {
            Err(PasskeyError::new(
                "NotAllowedError",
                "Windows answered with a passkey that is not registered for this vault.",
            ))
        };
        WebAuthNFreeAssertion(assertion);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_matches_rfc4648() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }

    // x64 sizes of the layouts the KeePass plugin uses (proven with a real iPhone passkey);
    // guards against accidental field changes.
    #[test]
    fn struct_sizes_match_plugin_layout() {
        assert_eq!(std::mem::size_of::<ClientData>(), 24);
        assert_eq!(std::mem::size_of::<Credential>(), 24);
        assert_eq!(std::mem::size_of::<HmacSecretSalt>(), 32);
        assert_eq!(std::mem::size_of::<GetAssertionOptions>(), 120);
        assert_eq!(std::mem::size_of::<MakeCredentialOptions>(), 104);
        assert_eq!(std::mem::size_of::<CredWithSalt>(), 24);
        assert_eq!(std::mem::size_of::<Extension>(), 24);
        assert_eq!(std::mem::size_of::<HmacSecretSaltValues>(), 24);
    }

    #[test]
    fn transport_labels() {
        assert_eq!(transport_label(TRANSPORT_USB), "Security key (USB)");
        assert_eq!(transport_label(TRANSPORT_HYBRID), "Phone passkey");
        assert_eq!(transport_label(TRANSPORT_INTERNAL), "Windows Hello");
        assert_eq!(transport_label(0), "Passkey");
    }

    #[test]
    fn api_is_available() {
        assert!(api_version() >= 6);
    }
}
