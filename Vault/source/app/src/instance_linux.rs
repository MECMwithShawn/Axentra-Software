//! Private Unix datagrams carry only Show, Lock and Exit, never credentials.
use std::{fs::File,os::unix::net::UnixDatagram,path::PathBuf};
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Command{Show,Lock,Exit}
impl Command{fn bytes(self)->&'static [u8]{match self{Self::Show=>b"show",Self::Lock=>b"lock",Self::Exit=>b"exit"}}}
pub fn command_from_args(args:impl IntoIterator<Item=String>)->Option<Command>{args.into_iter().find_map(|a|match a.as_str(){"--show"=>Some(Command::Show),"--lock"=>Some(Command::Lock),"--exit"=>Some(Command::Exit),_=>None})}
pub struct Instance{_lock:File,socket:UnixDatagram}
pub enum Start{Primary(Instance),Forwarded}
fn directory()->Result<PathBuf,String>{let identity=crate::settings::dir().ok_or("No settings directory")?;let mut h=0xcbf29ce484222325u64;for b in identity.as_os_str().as_encoded_bytes(){h^=*b as u64;h=h.wrapping_mul(0x100000001b3);}
    let path=crate::linux::runtime()?.join(format!("axv-{h:016x}"));crate::linux::private_dir(&path)?;Ok(path)}
fn try_start(command:Option<Command>)->Result<Start,String>{
    let dir=directory()?;let path=dir.join("commands");
    let Some(lock)=crate::linux::lock(&dir.join("instance.lock"))?else{
        let sock=UnixDatagram::unbound().map_err(|e|e.to_string())?;
        for _ in 0..20{if sock.send_to(command.unwrap_or(Command::Show).bytes(),&path).is_ok(){return Ok(Start::Forwarded)}std::thread::sleep(std::time::Duration::from_millis(50));}
        return Err("The running Vault did not accept the command".into())
    };
    match std::fs::remove_file(&path){Ok(())=>{},Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},Err(e)=>return Err(e.to_string())}
    let socket=UnixDatagram::bind(&path).map_err(|e|e.to_string())?;Ok(Start::Primary(Instance{_lock:lock,socket}))
}
pub fn start(command:Option<Command>)->Start{try_start(command).unwrap_or_else(|e|{eprintln!("Vault instance: {e}");std::process::exit(1)})}
impl Instance{pub fn listen(self,on_command:impl Fn(Command)+Send+'static){std::thread::spawn(move||{let instance=self;let mut bytes=[0u8;16];while let Ok(n)=instance.socket.recv(&mut bytes){let c=match &bytes[..n]{b"show"=>Command::Show,b"lock"=>Command::Lock,b"exit"=>Command::Exit,_=>continue};on_command(c);}});}}
