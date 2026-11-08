//! Linux kernel process protection. Desktop screen capture exclusion is unavailable.
pub fn restrict_process_access()->Result<(),String>{
    if unsafe{libc::prctl(libc::PR_SET_DUMPABLE,0,0,0,0)}!=0{return Err(std::io::Error::last_os_error().to_string())}Ok(())
}
pub fn apply_mitigations()->Vec<String>{
    let mut errors=Vec::new();
    let limit=libc::rlimit{rlim_cur:0,rlim_max:0};
    if unsafe{libc::setrlimit(libc::RLIMIT_CORE,&limit)}!=0{errors.push(format!("core dump limit: {}",std::io::Error::last_os_error()));}
    if unsafe{libc::prctl(libc::PR_SET_NO_NEW_PRIVS,1,0,0,0)}!=0{errors.push(format!("privilege protection: {}",std::io::Error::last_os_error()));}errors
}
pub fn capture_supported()->bool{false}
pub fn capture_hidden()->bool{false}
pub fn set_capture_hidden(_:bool){} // The disabled control cannot promise unsupported protection.
pub fn capture_hint()->&'static str{"Zorin does not provide per-window capture exclusion. This window can appear in screenshots and screen sharing."}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct SecurityStatus{protected:bool}
pub fn security_status()->SecurityStatus{SecurityStatus{protected:unsafe{libc::prctl(libc::PR_GET_DUMPABLE,0,0,0,0)}==0}}
impl SecurityStatus{
    pub fn summary(&self)->String{format!("Process memory protection: {} · Screen capture exclusion: unavailable on Linux",if self.protected{"on"}else{"not applied"})}
    pub fn warning(&self)->Option<&'static str>{(!self.protected).then_some("Linux process memory protection could not be applied.")}
}
