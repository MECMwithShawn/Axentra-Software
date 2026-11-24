use std::{fs::{self,File,OpenOptions},path::PathBuf,os::unix::{fs::{DirBuilderExt,MetadataExt,OpenOptionsExt},io::AsRawFd}};
fn xdg(variable:&str,fallback:&str)->PathBuf{std::env::var_os(variable).map(PathBuf::from).filter(|p|p.is_absolute()).unwrap_or_else(||PathBuf::from(std::env::var_os("HOME").expect("HOME is required")).join(fallback)).join("axentra")}
pub fn config_home()->PathBuf{std::env::var_os("AXENTRA_VAULT_HOME").map(PathBuf::from).unwrap_or_else(||xdg("XDG_CONFIG_HOME",".config"))}
pub fn data_home()->PathBuf{std::env::var_os("AXENTRA_VAULT_HOME").map(PathBuf::from).unwrap_or_else(||xdg("XDG_DATA_HOME",".local/share"))}
pub fn runtime()->Result<PathBuf,String>{let path=std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p|p.is_absolute()).unwrap_or_else(||std::env::temp_dir().join(format!("axentra-{}",unsafe{libc::geteuid()})));private_dir(&path)?;Ok(path)}
pub fn private_dir(path:&std::path::Path)->Result<(),String>{
    match fs::DirBuilder::new().mode(0o700).create(path){Ok(())=>{},Err(e)if e.kind()==std::io::ErrorKind::AlreadyExists=>{},Err(e)=>return Err(e.to_string())}
    let m=fs::symlink_metadata(path).map_err(|e|e.to_string())?;
    if !m.is_dir()||m.file_type().is_symlink()||m.uid()!=unsafe{libc::geteuid()}||m.mode()&0o077!=0{return Err("Unsafe runtime folder".into())}Ok(())
}
pub fn lock(path:&std::path::Path)->Result<Option<File>,String>{let file=OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(path).map_err(|e|e.to_string())?;
    if unsafe{libc::flock(file.as_raw_fd(),libc::LOCK_EX|libc::LOCK_NB)}!=0{let e=std::io::Error::last_os_error();return if e.raw_os_error()==Some(libc::EWOULDBLOCK){Ok(None)}else{Err(e.to_string())}}Ok(Some(file))}
pub fn tray_hosted()->bool{runtime().ok().and_then(|p|lock(&p.join(if std::env::var_os("AXENTRA_VAULT_HOME").is_some(){"axentra-suite-tray-test.lock"}else{"axentra-suite-tray.lock"})).ok()).is_some_and(|f|f.is_none())}
pub fn desktop_setting(schema:&str,key:&str)->String{std::process::Command::new("/usr/bin/gsettings").args(["get",schema,key]).output().ok().filter(|o|o.status.success()).map(|o|String::from_utf8_lossy(&o.stdout).trim().into()).unwrap_or_default()}
pub fn open_url(url:&str)->Result<(),String>{let mut child=std::process::Command::new("/usr/bin/xdg-open").arg(url).spawn().map_err(|e|e.to_string())?;std::thread::spawn(move||{let _=child.wait();});Ok(())}
