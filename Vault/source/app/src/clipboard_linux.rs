//! Clipboard ownership with timeout, zeroized storage and the desktop password-manager hint.
//!
//! The secret is served from its own thread with its own X connection, so a paste is answered at
//! once even while Vault's window is hidden, covered or minimized and its frames (and with them
//! the GTK loop in `logic`) are paused. That thread also clears the clipboard when the time is up.
//! GTK is the fallback when there is no X display.
use std::{cell::Cell,rc::Rc,sync::{Arc,atomic::{AtomicBool,Ordering},mpsc},thread,time::{Duration,Instant}};
use x11rb::{connection::Connection,protocol::{Event,xproto::{self,AtomEnum,ConnectionExt as _,CreateWindowAux,EventMask,PropMode,WindowClass}},rust_connection::RustConnection,wrapper::ConnectionExt as _,CURRENT_TIME,NONE};
use zeroize::Zeroizing;
pub const CLEAR_AFTER:Duration=Duration::from_secs(12);
struct Ownership(Rc<Cell<bool>>);
impl Drop for Ownership{fn drop(&mut self){self.0.set(false);}}
enum Owner{
    X{owned:Arc<AtomicBool>,clear:mpsc::Sender<mpsc::Sender<()>>},
    Gtk{clipboard:gtk::Clipboard,owned:Rc<Cell<bool>>},
}
pub struct PendingClear{owner:Owner,at:Instant}
impl PendingClear{
    pub fn due(&self)->bool{Instant::now()>=self.at}
    pub fn remaining(&self)->Duration{self.at.saturating_duration_since(Instant::now())}
    /// Waits briefly for the clipboard thread, so a copy made right after this keeps the clipboard.
    pub fn clear(self){match self.owner{
        Owner::X{owned,clear}=>if owned.load(Ordering::SeqCst){
            let (done,wait)=mpsc::channel();
            if clear.send(done).is_ok(){let _=wait.recv_timeout(Duration::from_millis(500));}
        },
        Owner::Gtk{clipboard,owned}=>if owned.get(){clipboard.clear();},
    }}
    #[cfg(test)]fn owned(&self)->bool{match &self.owner{Owner::X{owned,..}=>owned.load(Ordering::SeqCst),Owner::Gtk{owned,..}=>owned.get()}}
}
pub fn copy_secret(text:&str)->Option<PendingClear>{
    let at=Instant::now()+CLEAR_AFTER;
    x_owner(text,at).or_else(||gtk_owner(text)).map(|owner|PendingClear{owner,at})
}
#[derive(Clone,Copy)]struct Atoms{clipboard:u32,targets:u32,utf8:u32,plain:u32,hint:u32}
fn x_owner(text:&str,at:Instant)->Option<Owner>{
    let (conn,screen)=x11rb::connect(None).ok()?;
    let root=conn.setup().roots.get(screen)?.root;
    let window=conn.generate_id().ok()?;
    conn.create_window(0,window,root,0,0,1,1,0,WindowClass::INPUT_ONLY,0,&CreateWindowAux::new()).ok()?;
    let atom=|name:&str|conn.intern_atom(false,name.as_bytes()).ok()?.reply().ok().map(|r|r.atom);
    let atoms=Atoms{clipboard:atom("CLIPBOARD")?,targets:atom("TARGETS")?,utf8:atom("UTF8_STRING")?,
        plain:atom("text/plain;charset=utf-8")?,hint:atom("x-kde-passwordManagerHint")?};
    conn.set_selection_owner(window,atoms.clipboard,CURRENT_TIME).ok()?;
    if conn.get_selection_owner(atoms.clipboard).ok()?.reply().ok()?.owner!=window{return None}
    let secret=Zeroizing::new(text.as_bytes().to_vec());
    let owned=Arc::new(AtomicBool::new(true));let (clear,requests)=mpsc::channel();
    let flag=owned.clone();
    thread::Builder::new().name("vault-clipboard".into())
        .spawn(move||{serve(&conn,window,atoms,&secret,requests,at,&flag);flag.store(false,Ordering::SeqCst);}).ok()?;
    Some(Owner::X{owned,clear})
}
/// Answers paste requests until another app takes the clipboard, Vault clears it, or time is up.
fn serve(conn:&RustConnection,window:u32,a:Atoms,secret:&[u8],requests:mpsc::Receiver<mpsc::Sender<()>>,at:Instant,owned:&AtomicBool){
    let release=||{
        // Only if it's still ours: a later copy elsewhere must survive.
        if conn.get_selection_owner(a.clipboard).ok().and_then(|c|c.reply().ok()).is_some_and(|r|r.owner==window){
            let _=conn.set_selection_owner(NONE,a.clipboard,CURRENT_TIME);let _=conn.flush();
        }
    };
    loop{
        loop{match conn.poll_for_event(){
            Ok(Some(Event::SelectionRequest(request)))=>answer(conn,a,secret,request),
            Ok(Some(Event::SelectionClear(c)))if c.selection==a.clipboard=>return,
            Ok(Some(_))=>{},
            Ok(None)=>break,
            Err(_)=>return,
        }}
        if let Ok(done)=requests.try_recv(){release();owned.store(false,Ordering::SeqCst);let _=done.send(());return}
        if Instant::now()>=at{release();return}
        thread::sleep(Duration::from_millis(5));
    }
}
fn answer(conn:&RustConnection,a:Atoms,secret:&[u8],r:xproto::SelectionRequestEvent){
    let property=if r.property==NONE{r.target}else{r.property};
    let ok=if r.target==a.targets{
        conn.change_property32(PropMode::REPLACE,r.requestor,property,AtomEnum::ATOM,&[a.targets,a.utf8,a.plain,a.hint]).is_ok()
    }else if r.target==a.utf8||r.target==a.plain{
        conn.change_property8(PropMode::REPLACE,r.requestor,property,r.target,secret).is_ok()
    }else if r.target==a.hint{
        conn.change_property8(PropMode::REPLACE,r.requestor,property,a.hint,b"secret").is_ok()
    }else{false};
    let notify=xproto::SelectionNotifyEvent{response_type:xproto::SELECTION_NOTIFY_EVENT,sequence:0,time:r.time,
        requestor:r.requestor,selection:r.selection,target:r.target,property:if ok{property}else{NONE}};
    let _=conn.send_event(false,r.requestor,EventMask::NO_EVENT,notify);let _=conn.flush();
}
fn gtk_owner(text:&str)->Option<Owner>{
    if !gtk::is_initialized_main_thread(){return None}
    let clipboard=gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    let secret=Zeroizing::new(text.to_owned());
    let owned=Rc::new(Cell::new(true));let owner=Ownership(owned.clone());
    let targets=[gtk::TargetEntry::new("UTF8_STRING",gtk::TargetFlags::empty(),0),gtk::TargetEntry::new("text/plain;charset=utf-8",gtk::TargetFlags::empty(),0),gtk::TargetEntry::new("x-kde-passwordManagerHint",gtk::TargetFlags::empty(),1)];
    let ok=clipboard.set_with_data(&targets,move|_,data,info|{
        let _=&owner;
        if info==1{data.set(&gtk::gdk::Atom::intern("x-kde-passwordManagerHint"),8,b"secret");}else{data.set_text(&secret);}
    });
    ok.then_some(Owner::Gtk{clipboard,owned})
}
#[cfg(test)]mod tests{
    use super::*;
    /// Another process pastes while this thread is blocked, as Vault's is when its window is hidden.
    fn paste_elsewhere(target:&str)->String{
        let script=format!("import gi;gi.require_version('Gtk','3.0');from gi.repository import Gtk,Gdk\nc=Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)\nd=c.wait_for_contents(Gdk.Atom.intern('{target}',False))\nprint(d.get_data().decode() if d else '',end='')");
        let out=std::process::Command::new("timeout").args(["3","python3","-c",&script]).env("GDK_BACKEND","x11").output().unwrap();
        assert!(out.status.success(),"paste helper failed or hung: {out:?}");
        String::from_utf8(out.stdout).unwrap()
    }
    #[test]#[ignore="requires a disposable graphical session; run separately with one test thread"]
    fn clears_our_secret_without_clearing_a_later_copy(){
        gtk::init().unwrap();let first=copy_secret("synthetic secret").unwrap();assert!(first.owned());
        assert!(matches!(first.owner,Owner::X{..}));
        let started=Instant::now();
        assert_eq!(paste_elsewhere("UTF8_STRING"),"synthetic secret");
        assert!(started.elapsed()<Duration::from_secs(2),"paste took {:?}",started.elapsed());
        assert_eq!(paste_elsewhere("text/plain;charset=utf-8"),"synthetic secret");
        assert_eq!(paste_elsewhere("x-kde-passwordManagerHint"),"secret");
        // Another app copies: the thread lets go, and clearing ours later leaves theirs alone.
        let script="import gi;gi.require_version('Gtk','3.0');from gi.repository import Gtk,Gdk,GLib\nGtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).set_text('later copy',-1)\nGLib.timeout_add(3000,Gtk.main_quit);Gtk.main()";
        let mut other=std::process::Command::new("python3").args(["-c",script]).env("GDK_BACKEND","x11").spawn().unwrap();
        let deadline=Instant::now()+Duration::from_secs(2);
        while first.owned()&&Instant::now()<deadline{thread::sleep(Duration::from_millis(10));}
        assert!(!first.owned(),"another app's copy takes over");first.clear();
        assert_eq!(paste_elsewhere("UTF8_STRING"),"later copy");
        other.kill().unwrap();other.wait().unwrap();
        let clipboard=gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        let second=copy_secret("another secret").unwrap();
        let Owner::X{owned:flag,..}=&second.owner else{panic!("expected the X clipboard thread")};let flag=flag.clone();
        second.clear();assert!(!flag.load(Ordering::SeqCst),"clear waits for the thread to let go");
        assert_eq!(paste_elsewhere("UTF8_STRING"),"");
        let third=PendingClear{owner:x_owner("short-lived secret",Instant::now()+Duration::from_millis(200)).unwrap(),at:Instant::now()};
        assert_eq!(paste_elsewhere("UTF8_STRING"),"short-lived secret");
        thread::sleep(Duration::from_millis(400));
        assert!(!third.owned(),"the clipboard thread clears on time by itself");
        assert_eq!(paste_elsewhere("UTF8_STRING"),"");
        // GTK, used when there is no X display, must still copy and clear. (GTK is bound to the
        // first thread that starts it, so this shares the test above.)
        let owner=gtk_owner("gtk secret").unwrap();
        assert_eq!(clipboard.wait_for_text().unwrap(),"gtk secret");
        let pending=PendingClear{owner,at:Instant::now()};pending.clear();
        assert!(clipboard.wait_for_text().is_none_or(|s|s.is_empty()));
    }
}
