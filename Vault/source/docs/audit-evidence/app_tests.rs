use super::*;
use egui_kittest::{Harness,kittest::Queryable};
use vault_core::keepass::{Database,DatabaseKey,config::KdfConfig};
#[test]
fn audit_master_password_can_be_undone_after_lock() {
    let dir=std::env::temp_dir().join(format!("audit-master-{}",uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path=dir.join("synthetic.kdbx");
    let mut db=Database::new(); db.config.kdf_config=KdfConfig::Aes{rounds:1000};
    db.save(&mut std::fs::File::create(&path).unwrap(),DatabaseKey::new().with_password("audit-master-canary")).unwrap();
    let mut h=Harness::builder().with_size(egui::vec2(1280.0,800.0)).build_eframe(|cc| VaultApp::with_layout(cc,UiLayout::Classic));
    let mut unlock=Unlock::new(Some(path.display().to_string()),false);
    *unlock.password="audit-master-canary".into();
    h.state_mut().screen=Screen::Locked(Box::new(unlock));
    h.run_steps(4);
    h.get_by_label("Unlock vault").click();
    for _ in 0..200 {
        h.run_steps(1);
        if matches!(h.state().screen,Screen::Open(_)){break;}
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(h.state().screen,Screen::Open(_)));
    h.run_steps(3); h.get_by_label("Lock").click(); h.run_steps(3);
    let Screen::Locked(u)=&h.state().screen else {panic!("not locked")};
    assert!(u.password.is_empty());
    h.get_by_role(egui::accesskit::Role::PasswordInput).focus(); h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND,egui::Key::Z); h.run_steps(2);
    let Screen::Locked(u)=&h.state().screen else {panic!("not locked")};
    assert_eq!(&*u.password,"audit-master-canary");
    println!("CONFIRMED: real app unlock -> Lock -> Ctrl+Z restores previous master password in locked form.");
}


