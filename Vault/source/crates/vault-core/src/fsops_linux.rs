//! Linux atomic publishing, private files and filesystem facts.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path,PathBuf};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use crate::random_bytes;
const TEMP_PREFIX: &str = ".~axv-";
const TEMP_SUFFIX: &str = ".tmp";

/// A fresh temp file name, `.~axv-<16 hex>.tmp`.
pub fn temp_name() -> String {
    let mut n = [0u8; 8];
    random_bytes(&mut n);
    format!("{TEMP_PREFIX}{}{TEMP_SUFFIX}", hex::encode(n))
}

/// Whether `name` is exactly a name [`temp_name`] makes. Anything else is left alone.
pub fn is_temp_name(name: &str) -> bool {
    name.strip_prefix(TEMP_PREFIX)
        .and_then(|rest| rest.strip_suffix(TEMP_SUFFIX))
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// Creates `path` (failing if anything is there), writes `bytes` and flushes them to disk. A
/// partly written file is removed again.
pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path)?;
    if let Err(e) = f.write_all(bytes).and_then(|()| f.sync_all()) {
        drop(f);
        let _ = fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}

/// Writes `bytes` to a new, uniquely named temp file in `dir` (see [`write_new`]).
pub fn write_temp_in(dir: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let mut last = None;
    for _ in 0..4 {
        let path = dir.join(temp_name());
        match write_new(&path, bytes) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::ErrorKind::AlreadyExists.into()))
}

/// Writes `bytes` to a new temp file beside `path`, named `<path>.~axv-<16 hex>.tmp` so a
/// leftover can be traced to the file it was for (see [`write_new`]).
pub fn write_temp_beside(path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let mut last = None;
    for _ in 0..4 {
        let mut name = path.as_os_str().to_owned();
        name.push(temp_name());
        let tmp = PathBuf::from(name);
        match write_new(&tmp, bytes) {
            Ok(()) => return Ok(tmp),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::ErrorKind::AlreadyExists.into()))
}

fn cpath(path:&Path)->io::Result<std::ffi::CString>{std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(|_|io::ErrorKind::InvalidInput.into())}
fn sync_parent(path:&Path)->io::Result<()>{File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()}
pub fn move_no_replace(from:&Path,to:&Path)->io::Result<()>{
    let (a,b)=(cpath(from)?,cpath(to)?);
    if unsafe{libc::renameat2(libc::AT_FDCWD,a.as_ptr(),libc::AT_FDCWD,b.as_ptr(),libc::RENAME_NOREPLACE)}!=0{
        let e=io::Error::last_os_error();
        // NFS and many FUSE filesystems lack RENAME_NOREPLACE. A hard link also refuses to
        // replace an existing file; then the source name goes.
        if !matches!(e.raw_os_error(),Some(libc::EINVAL|libc::ENOSYS|libc::EOPNOTSUPP)){return Err(e)}
        fs::hard_link(from,to)?;
        if let Err(e)=fs::remove_file(from){let _=fs::remove_file(to);return Err(e)}
    }
    sync_parent(to)?;if from.parent()!=to.parent(){sync_parent(from)?;}Ok(())
}
pub fn move_replace(from:&Path,to:&Path)->io::Result<()>{fs::rename(from,to)?;sync_parent(to)?;if from.parent()!=to.parent(){sync_parent(from)?;}Ok(())}
pub fn write_replacing(path:&Path,bytes:&[u8])->io::Result<()>{let tmp=write_temp_beside(path,bytes)?;move_replace(&tmp,path).inspect_err(|_|{let _=fs::remove_file(&tmp);})}
pub fn set_readonly(path:&Path,readonly:bool)->io::Result<()>{
    let m=fs::symlink_metadata(path)?;if m.file_type().is_symlink(){return Err(io::ErrorKind::InvalidInput.into())}
    let mode=if readonly{m.mode()&!0o222}else{m.mode()|0o200};
    fs::set_permissions(path,fs::Permissions::from_mode(mode))
}
pub fn remove_file(path:&Path)->io::Result<bool>{match fs::remove_file(path){Ok(())=>Ok(true),Err(e)if e.kind()==io::ErrorKind::NotFound=>Ok(false),Err(e)=>Err(e)}}
pub fn is_reparse_point(meta:&fs::Metadata)->bool{meta.file_type().is_symlink()}
pub fn free_space(dir:&Path)->Option<u64>{let p=cpath(dir).ok()?;let mut stat=std::mem::MaybeUninit::<libc::statvfs>::uninit();if unsafe{libc::statvfs(p.as_ptr(),stat.as_mut_ptr())}!=0{return None}let stat=unsafe{stat.assume_init()};Some(stat.f_bavail.saturating_mul(stat.f_frsize))}
struct Mount{root:PathBuf,kind:String,device:String}
fn unescape(s:&str)->String{s.replace("\\040"," ").replace("\\011","\t").replace("\\012","\n").replace("\\134","\\")}
fn mount(path:&Path)->Option<Mount>{
    let mut parent=path.to_path_buf();while !parent.exists(){if !parent.pop(){return None}}
    let path=fs::canonicalize(parent).ok()?;let table=fs::read_to_string("/proc/self/mountinfo").ok()?;
    table.lines().filter_map(|line|{let (left,right)=line.split_once(" - ")?;let fields:Vec<_>=left.split_whitespace().collect();let mut fs=right.split_whitespace();let kind=fs.next()?.to_owned();let device=unescape(fs.next()?);let root=PathBuf::from(unescape(fields.get(4)?));path.starts_with(&root).then_some(Mount{root,kind,device})}).max_by_key(|m|m.root.as_os_str().len())
}
pub fn volume_path(path:&Path)->Option<String>{mount(path).map(|m|m.root.display().to_string())}
pub fn same_volume(a:&Path,b:&Path)->bool{match(fs::metadata(a),fs::metadata(b)){(Ok(a),Ok(b))=>a.dev()==b.dev(),_=>match(mount(a),mount(b)){(Some(a),Some(b))=>a.device==b.device,_=>true}}}
pub fn is_network(path:&Path)->bool{mount(path).is_some_and(|m|matches!(m.kind.as_str(),"nfs"|"nfs4"|"cifs"|"smb3"|"fuse.sshfs"|"9p"|"ceph"|"glusterfs"))}
pub fn is_removable(path:&Path)->bool{
    let Some(m)=mount(path)else{return false};let Some(name)=Path::new(&m.device).file_name()else{return false};let Ok(mut p)=fs::canonicalize(Path::new("/sys/class/block").join(name))else{return false};
    loop{if fs::read_to_string(p.join("removable")).is_ok_and(|s|s.trim()=="1"){return true}if !p.pop()||p==Path::new("/sys"){return false}}
}
#[derive(Debug,Clone,PartialEq,Eq)]pub struct VolumeInfo{pub file_system:String,pub persistent_acls:bool}
pub fn volume_info(path:&Path)->Option<VolumeInfo>{let m=mount(path)?;let permissions=matches!(m.kind.as_str(),"ext2"|"ext3"|"ext4"|"btrfs"|"xfs"|"zfs"|"tmpfs"|"overlay");Some(VolumeInfo{file_system:m.kind,persistent_acls:permissions})}
pub fn create_dir_private(path:&Path)->io::Result<()>{
    fs::DirBuilder::new().mode(0o700).create(path)?;
    let m=fs::symlink_metadata(path)?;
    if !m.is_dir()||m.mode()&0o077!=0||m.uid()!=unsafe{libc::geteuid()}{return Err(io::Error::new(io::ErrorKind::PermissionDenied,"Filesystem does not enforce private directory permissions"))}Ok(())
}
#[cfg(test)]mod tests{
    use super::*;
    fn root()->PathBuf{let p=std::env::temp_dir().join(temp_name());create_dir_private(&p).unwrap();p}
    #[test]fn publishing_is_atomic_and_never_clobbers(){let r=root();let(a,b)=(r.join("a"),r.join("b"));write_new(&a,b"old").unwrap();write_new(&b,b"new").unwrap();assert_eq!(move_no_replace(&b,&a).unwrap_err().kind(),io::ErrorKind::AlreadyExists);assert_eq!(fs::read(&a).unwrap(),b"old");assert_eq!(fs::read(&b).unwrap(),b"new");move_replace(&b,&a).unwrap();assert_eq!(fs::read(&a).unwrap(),b"new");fs::remove_dir_all(r).unwrap();}
    #[test]fn secrets_are_private_and_symlinks_not_followed(){let r=root();let a=r.join("a");write_new(&a,b"secret").unwrap();assert_eq!(fs::metadata(&a).unwrap().mode()&0o777,0o600);let link=r.join("link");std::os::unix::fs::symlink(&a,&link).unwrap();assert!(write_new(&link,b"overwrite").is_err());assert!(set_readonly(&link,true).is_err());remove_file(&link).unwrap();assert_eq!(fs::read(&a).unwrap(),b"secret");set_readonly(&a,true).unwrap();assert_eq!(fs::metadata(&a).unwrap().mode()&0o777,0o400);set_readonly(&a,false).unwrap();assert_eq!(fs::metadata(&a).unwrap().mode()&0o777,0o600);fs::remove_dir_all(r).unwrap();}
    #[test]fn mount_and_space_facts(){let r=root();assert!(volume_info(&r).is_some());assert!(volume_path(&r).is_some());assert!(free_space(&r).unwrap()>0);assert!(same_volume(&r,&r));assert!(is_temp_name(&temp_name()));fs::remove_dir_all(r).unwrap();}
}
