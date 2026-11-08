use super::*;
use vault_core::keepass::{Database, config::KdfConfig};
fn fixture() -> Session {
    let dir = std::env::temp_dir().join(format!("audit-session-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("synthetic.kdbx");
    let mut db = Database::new();
    db.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
    db.root_mut().add_entry().set_protected(fields::PASSWORD, "audit-only-password");
    let credentials = Credentials { password: Some(Zeroizing::new("audit-master".into())), key_file: None };
    db.save(&mut fs::File::create(&path).unwrap(), credentials.to_key().unwrap()).unwrap();
    Session::new(Vault::open(&path, credentials).unwrap())
}
#[test]
fn audit_idle_lock_fails_open_on_save_conflict() {
    let mut session = fixture();
    session.vault.mark_dirty();
    use std::io::Write;
    fs::OpenOptions::new().append(true).open(session.vault.path()).unwrap().write_all(b"external-change").unwrap();
    session.last_activity = Instant::now() - IDLE_LOCK - Duration::from_secs(1);
    let (action, _) = session.frame_logic(&egui::Context::default());
    assert!(matches!(action, SessionAction::None));
    assert!(session.last_activity.elapsed() < Duration::from_secs(1));
    assert!(session.error.is_none());
    println!("CONFIRMED: overdue dirty session stays open and silently resets idle timer after save conflict.");
}
#[test]
fn audit_rekey_leaves_password_only_backup() {
    let mut session = fixture();
    let old = session.vault.credentials().clone();
    session.vault.set_credentials(Credentials { password: old.password.clone(), key_file: Some(Zeroizing::new(vec![42;32])) });
    session.vault.save().unwrap();
    assert!(Vault::open(session.vault.path(), old.clone()).is_err());
    assert!(Vault::open(&with_suffix(session.vault.path(), ".bak"), old).is_ok());
    println!("CONFIRMED: current DB needs two factors; automatically-created .bak needs password only.");
}
#[test]
fn audit_removed_ring_key_opens_current_database() {
    use vault_core::envelope::Envelope;
    let mut session = fixture();
    let key = [42;32]; let prf_a = [11;32]; let prf_b = [12;32];
    session.vault.set_credentials(Credentials { password: Some(Zeroizing::new("audit-master".into())), key_file: Some(Zeroizing::new(key.to_vec())) });
    session.vault.save().unwrap();
    let path = envelope::path_for(session.vault.path());
    let mut ring = KeyRing { keys: vec![
        RingKey { label:"removed".into(), envelope:Envelope::seal("localhost", &[1], [3;32], &prf_a, &key) },
        RingKey { label:"kept".into(), envelope:Envelope::seal("localhost", &[2], [3;32], &prf_b, &key) }
    ]};
    write_ring(&path, &ring).unwrap();
    ring.keys.remove(0);
    write_ring(&path, &ring).unwrap();
    session.vault.db.root_mut().add_entry().set_protected(fields::PASSWORD, "created-after-removal");
    session.vault.mark_dirty(); session.vault.save().unwrap();
    let old = KeyRing::parse(&fs::read_to_string(with_suffix(&path, ".old")).unwrap()).unwrap();
    let recovered = old.keys[0].envelope.open(&prf_a).unwrap();
    let opened = Vault::open(session.vault.path(), Credentials { password: Some(Zeroizing::new("audit-master".into())), key_file: Some(Zeroizing::new(recovered.to_vec())) }).unwrap();
    assert_eq!(opened.db.num_entries(), 2);
    println!("CONFIRMED: removed key from .old opens later database contents, given master password.");
}
#[test]
fn audit_hex_key_with_newline_is_misidentified_as_raw() {
    let mut session = fixture();
    let bytes = format!("{}\n", "2a".repeat(32)).into_bytes();
    let original = Credentials { password: Some(Zeroizing::new("audit-master".into())), key_file: Some(Zeroizing::new(bytes.clone())) };
    session.vault.set_credentials(original.clone()); session.vault.save().unwrap();
    assert!(Vault::open(session.vault.path(), original).is_ok());
    let extracted = keyfile::raw_key(&bytes).unwrap();
    let wrong = Credentials { password: Some(Zeroizing::new("audit-master".into())), key_file: Some(Zeroizing::new(extracted.to_vec())) };
    assert!(Vault::open(session.vault.path(), wrong).is_err());
    println!("CONFIRMED: raw_key trims hex key; KeePass hashes 65-byte file, so added passkey wraps wrong key.");
}
#[test]
fn audit_password_undo_survives_session_drop() {
    let ctx = egui::Context::default();
    let id = egui::Id::new("audit-password");
    let mut session = fixture();
    session.start_new_entry();
    *session.editor.as_mut().unwrap().form.password = "audit-undo-canary".into();
    for frame in 0..3 {
        let raw = egui::RawInput { time:Some(frame as f64), ..Default::default() };
        let mut audit_output = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let out = egui::TextEdit::singleline(&mut *session.editor.as_mut().unwrap().form.password).password(true).id(id).show(ui);
                out.response.request_focus();
            });
        });
        audit_output.textures_delta.clear();
    }
    drop(session);
    let state = egui::TextEdit::load_state(&ctx, id).unwrap();
    let mut undo = state.undoer();
    let recovered = undo.undo(&(egui::text::CCursorRange::default(), String::new())).unwrap();
    assert_eq!(recovered.1, "audit-undo-canary");
    println!("CONFIRMED: plaintext password recovered from egui context after Session and Zeroizing form are dropped.");
}
#[test]
fn audit_revealed_text_emits_unmanaged_clipboard_command() {
    let ctx = egui::Context::default(); let id=egui::Id::new("audit-revealed");
    let mut secret = Zeroizing::new(String::from("audit-copy-canary"));
    let draw = |ui: &mut egui::Ui, secret: &mut String| {
        egui::CentralPanel::default().show(ui, |ui| {
            egui::TextEdit::singleline(secret).password(false).id(id).show(ui).response.request_focus();
        });
    };
    let mut audit_output = ctx.run_ui(Default::default(), |ui| draw(ui, &mut secret));
    audit_output.textures_delta.clear();
    let mut state=egui::TextEdit::load_state(&ctx,id).unwrap();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0),egui::text::CCursor::new(secret.len()))));
    state.store(&ctx,id);
    let mut output=ctx.run_ui(egui::RawInput{events:vec![egui::Event::Copy],..Default::default()},|ui|draw(ui,&mut secret));
    assert!(output.platform_output.commands.iter().any(|c| matches!(c,egui::OutputCommand::CopyText(s) if s=="audit-copy-canary")));
    output.textures_delta.clear();
    println!("CONFIRMED: revealed TextEdit Ctrl+C emits egui CopyText outside protected clipboard path.");
}
#[test]
fn audit_key_ring_accepts_oversized_credential() {
    use vault_core::envelope::Envelope;
    let env=Envelope::seal("localhost",&vec![1;1024*1024],[2;32],&[3;32],&[4;32]);
    let ring=KeyRing::parse(&env.serialize()).unwrap();
    assert_eq!(ring.keys[0].envelope.credential_id.len(),1024*1024);
    println!("CONFIRMED: parser accepts one-megabyte credential ID with no resource budget.");
}

