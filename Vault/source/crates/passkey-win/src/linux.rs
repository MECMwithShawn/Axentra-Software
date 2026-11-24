//! Native FIDO2 hmac-secret with the WebAuthn PRF salt domain. USB/NFC authenticators.
use std::ffi::{c_void,c_char,CStr,CString};
use zeroize::Zeroizing;
use sha2::{Sha256,Digest};
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
        "Platform authenticator"
    } else {
        "Passkey"
    }
}

/// A registered passkey and the PRF input its envelope was sealed with.
pub struct PrfCredential<'a> {
    pub credential_id: &'a [u8],
    pub prf_input: &'a [u8; 32],
}

#[link(name="fido2")]
unsafe extern "C" {
    fn fido_init(flags:i32);
    fn fido_strerr(code:i32)->*const c_char;
    fn fido_dev_info_new(n:usize)->*mut c_void;
    fn fido_dev_info_free(list:*mut *mut c_void,n:usize);
    fn fido_dev_info_manifest(list:*mut c_void,capacity:usize,count:*mut usize)->i32;
    fn fido_dev_info_ptr(list:*const c_void,index:usize)->*const c_void;
    fn fido_dev_info_path(info:*const c_void)->*const c_char;
    fn fido_dev_info_product_string(info:*const c_void)->*const c_char;
    fn fido_dev_new()->*mut c_void;
    fn fido_dev_free(dev:*mut *mut c_void);
    fn fido_dev_open(dev:*mut c_void,path:*const c_char)->i32;
    fn fido_dev_close(dev:*mut c_void)->i32;
    fn fido_dev_set_timeout(dev:*mut c_void,millis:i32)->i32;
    fn fido_dev_has_pin(dev:*const c_void)->bool;
    fn fido_dev_supports_uv(dev:*const c_void)->bool;
    fn fido_dev_make_cred(dev:*mut c_void,cred:*mut c_void,pin:*const c_char)->i32;
    fn fido_dev_get_assert(dev:*mut c_void,assert:*mut c_void,pin:*const c_char)->i32;
    fn fido_cred_new()->*mut c_void;
    fn fido_cred_free(cred:*mut *mut c_void);
    fn fido_cred_set_type(cred:*mut c_void,kind:i32)->i32;
    fn fido_cred_set_rp(cred:*mut c_void,id:*const c_char,name:*const c_char)->i32;
    fn fido_cred_set_user(cred:*mut c_void,id:*const u8,len:usize,name:*const c_char,display:*const c_char,icon:*const c_char)->i32;
    fn fido_cred_set_clientdata_hash(cred:*mut c_void,hash:*const u8,len:usize)->i32;
    fn fido_cred_set_extensions(cred:*mut c_void,extensions:i32)->i32;
    fn fido_cred_set_rk(cred:*mut c_void,value:i32)->i32;
    fn fido_cred_set_uv(cred:*mut c_void,value:i32)->i32;
    fn fido_cred_exclude(cred:*mut c_void,id:*const u8,len:usize)->i32;
    fn fido_cred_verify(cred:*const c_void)->i32;
    fn fido_cred_verify_self(cred:*const c_void)->i32;
    fn fido_cred_x5c_len(cred:*const c_void)->usize;
    fn fido_cred_id_ptr(cred:*const c_void)->*const u8;
    fn fido_cred_id_len(cred:*const c_void)->usize;
    fn fido_assert_new()->*mut c_void;
    fn fido_assert_free(assert:*mut *mut c_void);
    fn fido_assert_set_rp(assert:*mut c_void,id:*const c_char)->i32;
    fn fido_assert_set_clientdata_hash(assert:*mut c_void,hash:*const u8,len:usize)->i32;
    fn fido_assert_set_extensions(assert:*mut c_void,extensions:i32)->i32;
    fn fido_assert_set_hmac_salt(assert:*mut c_void,salt:*const u8,len:usize)->i32;
    fn fido_assert_set_up(assert:*mut c_void,value:i32)->i32;
    fn fido_assert_set_uv(assert:*mut c_void,value:i32)->i32;
    fn fido_assert_allow_cred(assert:*mut c_void,id:*const u8,len:usize)->i32;
    fn fido_assert_count(assert:*const c_void)->usize;
    fn fido_assert_flags(assert:*const c_void,index:usize)->u8;
    fn fido_assert_id_ptr(assert:*const c_void,index:usize)->*const u8;
    fn fido_assert_id_len(assert:*const c_void,index:usize)->usize;
    fn fido_assert_hmac_secret_ptr(assert:*const c_void,index:usize)->*const u8;
    fn fido_assert_hmac_secret_len(assert:*const c_void,index:usize)->usize;
}
fn check(code:i32)->Result<(),PasskeyError>{if code==0{return Ok(())}let name=if matches!(code,0x2d|0x2f|0x3a){"NotAllowedError"}else{"AuthenticatorError"};let text=unsafe{CStr::from_ptr(fido_strerr(code))}.to_string_lossy();Err(PasskeyError::new(name,format!("Security key: {text}.")))}
fn cstr(value:&str)->Result<CString,PasskeyError>{CString::new(value).map_err(|_|PasskeyError::new("DataError","Invalid authenticator text"))}
fn challenge()->Result<[u8;32],PasskeyError>{let mut v=[0;32];getrandom::fill(&mut v).map_err(|e|PasskeyError::new("RandomError",e.to_string()))?;Ok(v)}
fn prf_salt(input:&[u8;32])->[u8;32]{let mut h=Sha256::new();h.update(b"WebAuthn PRF\0");h.update(input);h.finalize().into()}
struct Device(*mut c_void);
impl Drop for Device{fn drop(&mut self){unsafe{fido_dev_close(self.0);fido_dev_free(&mut self.0);}}}
struct Credential(*mut c_void);
impl Drop for Credential{fn drop(&mut self){unsafe{fido_cred_free(&mut self.0)}}}
struct Assertion(*mut c_void);
impl Drop for Assertion{fn drop(&mut self){unsafe{fido_assert_free(&mut self.0)}}}
struct Devices(*mut c_void);
impl Drop for Devices{fn drop(&mut self){unsafe{fido_dev_info_free(&mut self.0,64)}}}
fn device()->Result<Device,PasskeyError>{
    static INIT:std::sync::Once=std::sync::Once::new();INIT.call_once(||unsafe{fido_init(0)});
    let list=Devices(unsafe{fido_dev_info_new(64)});if list.0.is_null(){return Err(PasskeyError::new("MemoryError","Cannot enumerate security keys"))}
    let mut count=0;check(unsafe{fido_dev_info_manifest(list.0,64,&mut count)})?;
    if count==0{return Err(PasskeyError::new("NotFoundError","Connect a FIDO2 security key with hmac-secret support. Phone passkeys and Windows Hello are not available through this Linux adapter; the backup key file remains supported."))}
    let mut items=Vec::new();for i in 0..count.min(64){let info=unsafe{fido_dev_info_ptr(list.0,i)};if info.is_null(){continue}let path=unsafe{fido_dev_info_path(info)};if path.is_null(){continue}let label=unsafe{fido_dev_info_product_string(info)};items.push((unsafe{CStr::from_ptr(path)}.to_owned(),if label.is_null(){"Security key".into()}else{unsafe{CStr::from_ptr(label)}.to_string_lossy().into_owned()}));}
    if items.is_empty(){return Err(PasskeyError::new("NotFoundError","No accessible security keys"))}
    let choice=if items.len()==1{0}else{
        let mut c=std::process::Command::new("/usr/bin/zenity");c.args(["--list","--title=Axentra Vault — security key","--text=Choose a security key","--column=Number","--column=Key"]);for (i,(_,label))in items.iter().enumerate(){c.arg((i+1).to_string()).arg(label);}
        let out=c.output().map_err(|e|PasskeyError::new("DialogError",e.to_string()))?;
        if !out.status.success(){return Err(PasskeyError::new("AbortError","Security key selection cancelled"))}
        String::from_utf8_lossy(&out.stdout).trim().parse::<usize>().ok().and_then(|n|n.checked_sub(1)).filter(|n|*n<items.len()).ok_or_else(||PasskeyError::new("DataError","Invalid key selection"))?
    };
    let dev=Device(unsafe{fido_dev_new()});if dev.0.is_null(){return Err(PasskeyError::new("MemoryError","Cannot open security key"))}
    check(unsafe{fido_dev_open(dev.0,items[choice].0.as_ptr())})?;check(unsafe{fido_dev_set_timeout(dev.0,TIMEOUT_MS as i32)})?;Ok(dev)
}
fn pin(device:&Device)->Result<Option<Zeroizing<Vec<u8>>>,PasskeyError>{
    // Use an existing key PIN for user verification. Never retry an invalid PIN automatically.
    if unsafe{fido_dev_has_pin(device.0)}{
        let output=std::process::Command::new("/usr/bin/zenity").args(["--password","--title=Axentra Vault — security key PIN","--timeout=120"]).output().map_err(|e|PasskeyError::new("DialogError",e.to_string()))?;
        let mut pin=Zeroizing::new(output.stdout);if !output.status.success(){return Err(PasskeyError::new("AbortError","Security key PIN entry cancelled"))}
        while matches!(pin.last(),Some(b'\n'|b'\r')){pin.pop();}
        if pin.is_empty()||pin.contains(&0){return Err(PasskeyError::new("DataError","A security key PIN is required"))}pin.push(0);Ok(Some(pin))
    }else if unsafe{fido_dev_supports_uv(device.0)}{Ok(None)}else{Err(PasskeyError::new("VerificationRequired","Set a PIN on this FIDO2 security key using its management tool, then try again. User verification is required."))}
}
fn pin_ptr(pin:&Option<Zeroizing<Vec<u8>>>)->*const c_char{pin.as_ref().map_or(std::ptr::null(),|p|p.as_ptr().cast())}
pub fn foreground_window()->Hwnd{std::ptr::null_mut()}
pub fn register(_:Hwnd,rp_id:&str,rp_name:&str,user_id:&[u8],user_name:&str,exclude:&[&[u8]])->Result<Registration,PasskeyError>{
    if user_id.is_empty()||user_id.len()>64{return Err(PasskeyError::new("DataError","Invalid authenticator user identifier"))}
    let (rp,name,user)=(cstr(rp_id)?,cstr(rp_name)?,cstr(user_name)?);let dev=device()?;let pin=pin(&dev)?;
    let cred=Credential(unsafe{fido_cred_new()});if cred.0.is_null(){return Err(PasskeyError::new("MemoryError","Cannot create credential request"))}
    let hash=challenge()?;
    unsafe{
        check(fido_cred_set_type(cred.0,COSE_ES256))?;check(fido_cred_set_rp(cred.0,rp.as_ptr(),name.as_ptr()))?;
        check(fido_cred_set_user(cred.0,user_id.as_ptr(),user_id.len(),user.as_ptr(),user.as_ptr(),std::ptr::null()))?;
        check(fido_cred_set_clientdata_hash(cred.0,hash.as_ptr(),hash.len()))?;check(fido_cred_set_extensions(cred.0,1))?;
        check(fido_cred_set_rk(cred.0,2))?;check(fido_cred_set_uv(cred.0,2))?;
        for id in exclude{check(fido_cred_exclude(cred.0,id.as_ptr(),id.len()))?}
        check(fido_dev_make_cred(dev.0,cred.0,pin_ptr(&pin)))?;
        check(if fido_cred_x5c_len(cred.0)>0{fido_cred_verify(cred.0)}else{fido_cred_verify_self(cred.0)})?;
        let len=fido_cred_id_len(cred.0);let ptr=fido_cred_id_ptr(cred.0);if ptr.is_null()||len==0||len>4096{return Err(PasskeyError::new("DataError","Invalid credential identifier"))}
        // The app immediately evaluates PRF and refuses to persist a key that cannot derive it.
        Ok(Registration{credential_id:std::slice::from_raw_parts(ptr,len).to_vec(),prf_enabled:true,transport:TRANSPORT_USB})
    }
}
pub fn evaluate_prf(hwnd:Hwnd,rp_id:&str,id:&[u8],input:&[u8;32])->Result<Zeroizing<[u8;32]>,PasskeyError>{evaluate_prf_any(hwnd,rp_id,&[PrfCredential{credential_id:id,prf_input:input}]).map(|(_,v)|v)}
pub fn evaluate_prf_any(_:Hwnd,rp_id:&str,creds:&[PrfCredential])->Result<(usize,Zeroizing<[u8;32]>),PasskeyError>{
    if creds.is_empty()||creds.len()>64||creds.iter().any(|c|c.credential_id.is_empty()||c.credential_id.len()>4096){return Err(PasskeyError::new("DataError","Invalid passkey list"))}
    let rp=cstr(rp_id)?;let dev=device()?;let pin=pin(&dev)?;
    let mut groups=std::collections::BTreeMap::<[u8;32],Vec<usize>>::new();for(i,c)in creds.iter().enumerate(){groups.entry(prf_salt(c.prf_input)).or_default().push(i);}
    for(salt,indices)in groups{
        let a=Assertion(unsafe{fido_assert_new()});if a.0.is_null(){return Err(PasskeyError::new("MemoryError","Cannot create assertion"))}let hash=challenge()?;
        unsafe{
            check(fido_assert_set_rp(a.0,rp.as_ptr()))?;check(fido_assert_set_clientdata_hash(a.0,hash.as_ptr(),hash.len()))?;
            check(fido_assert_set_extensions(a.0,1))?;check(fido_assert_set_hmac_salt(a.0,salt.as_ptr(),salt.len()))?;check(fido_assert_set_up(a.0,2))?;check(fido_assert_set_uv(a.0,2))?;
            for i in &indices{let id=creds[*i].credential_id;check(fido_assert_allow_cred(a.0,id.as_ptr(),id.len()))?;}
            let code=fido_dev_get_assert(dev.0,a.0,pin_ptr(&pin));if code==0x2e{continue}check(code)?;
            if fido_assert_count(a.0)==0||fido_assert_flags(a.0,0)&5!=5{return Err(PasskeyError::new("VerificationRequired","The key did not verify user presence and identity"))}
            let id=fido_assert_id_ptr(a.0,0);let len=fido_assert_id_len(a.0,0);if id.is_null()||len==0||len>4096{return Err(PasskeyError::new("DataError","Missing credential identifier"))}
            let id=std::slice::from_raw_parts(id,len);let index=indices.into_iter().find(|i|creds[*i].credential_id==id).ok_or_else(||PasskeyError::new("DataError","Unrequested credential returned"))?;
            let ptr=fido_assert_hmac_secret_ptr(a.0,0);if ptr.is_null()||fido_assert_hmac_secret_len(a.0,0)!=32{return Err(PasskeyError::new("NotSupportedError","This authenticator did not return a PRF secret"))}
            let mut value=Zeroizing::new([0;32]);value.copy_from_slice(std::slice::from_raw_parts(ptr,32));return Ok((index,value));
        }
    }Err(PasskeyError::new("NotFoundError","This key does not hold a passkey for this vault"))
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn prf_is_domain_separated_from_raw_hmac_secret(){let input=[0u8;32];let mut expected=b"WebAuthn PRF\0".to_vec();expected.extend(input);assert_eq!(prf_salt(&input).as_slice(),Sha256::digest(expected).as_slice());assert_ne!(prf_salt(&input),input);}
    #[test]fn invalid_lists_fail_before_touching_hardware(){assert!(evaluate_prf_any(std::ptr::null_mut(),"axentra.local",&[]).is_err());assert!(evaluate_prf(std::ptr::null_mut(),"axentra.local",&[],&[0;32]).is_err());}
}
