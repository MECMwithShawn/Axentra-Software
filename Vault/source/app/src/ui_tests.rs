//! End-to-end UI tests: drive the real app headlessly (egui_kittest) against a throwaway
//! database: unlock, ranked search, views, archive/restore, previous passwords, edit, new entry,
//! delete, save, lock, external-change protection and panel sizing. Clipboard, file dialogs and
//! passkeys need the real desktop and are left to manual testing.

use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{Duration, Instant};

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use vault_core::categorize;
use vault_core::keepass::db::{GroupId, Times, fields};
use vault_core::keepass::{Database, DatabaseKey, config::KdfConfig};
use vault_core::organize;
use vault_core::vault::{Credentials, Vault};
use zeroize::Zeroizing;

use crate::app::VaultApp;

const PASSWORD: &str = "test-password";

fn temp_dir(name: &str) -> PathBuf {
    let mut n = [0u8; 6];
    vault_core::random_bytes(&mut n);
    let d = std::env::temp_dir().join(format!(
        "axv-ui-{name}-{}",
        n.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Keep the app's settings out of the real %APPDATA%, and its backups out of the real
/// %LOCALAPPDATA%. `settings` calls this itself in tests.
pub(crate) fn isolate_settings() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        std::env::set_var("APPDATA", temp_dir("appdata"));
        std::env::set_var("LOCALAPPDATA", temp_dir("localappdata"));
    });
}

fn creds() -> Credentials {
    Credentials {
        password: Some(Zeroizing::new(PASSWORD.into())),
        key_file: None,
    }
}

/// The same traps as the demo database, with a fast KDF.
fn make_db(dir: &Path) -> PathBuf {
    let mut db = Database::new();
    db.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
    let group = |db: &mut Database, name: &str| -> GroupId {
        let mut r = db.root_mut();
        let mut g = r.add_group();
        g.name = name.into();
        g.id()
    };
    let email = group(&mut db, "Email");
    let bank = group(&mut db, "Banking");
    let bin = group(&mut db, "Recycle Bin");
    db.meta.recyclebin_enabled = Some(true);
    db.meta.recyclebin_uuid = Some(bin.uuid());

    let add = |db: &mut Database,
               g: GroupId,
               title: &str,
               user: &str,
               pw: &str,
               url: &str,
               notes: &str| {
        let mut grp = db.group_mut(g).unwrap();
        let mut e = grp.add_entry();
        e.set_unprotected(fields::TITLE, title);
        e.set_unprotected(fields::USERNAME, user);
        e.set_protected(fields::PASSWORD, pw);
        e.set_unprotected(fields::URL, url);
        e.set_unprotected(fields::NOTES, notes);
        e.times.last_modification = Some(Times::now());
        e.id()
    };
    add(
        &mut db,
        email,
        "Outlook",
        "shawn@outlook.com",
        "Kp7#vQ2!mZ9@rT4$wX8&",
        "https://outlook.live.com/mail",
        "",
    );
    add(
        &mut db,
        email,
        "Gmail",
        "shawn@gmail.com",
        "summer2019",
        "https://mail.google.com",
        "Recovery address is my Outlook account",
    );
    add(
        &mut db,
        bank,
        "Chase",
        "shawnb",
        "summer2019",
        "https://chase.com",
        "",
    );
    add(
        &mut db,
        bank,
        "Lookout Mobile Security",
        "shawn",
        "Lm4$Tr8!Qw2#Zx6@Pv9",
        "https://lookout.com",
        "",
    );
    let cu = add(
        &mut db,
        bank,
        "Credit union",
        "shawn.b",
        "first-password",
        "https://cu.example",
        "",
    );
    for pw in ["second-password", "Cu9#dL4!pW7@kQ2$mZ5&"] {
        organize::edit_with_history(&mut db, cu, |e| e.set_protected(fields::PASSWORD, pw));
    }
    let long = "Client ID d48e7898718647a0aa28cd5b9deeea97d48e7898718647a0aa28cd5b9deeea97d48e7898718647a0aa28cd5b9deeea97";
    add(
        &mut db,
        email,
        "Spotify Developer",
        "shawn@outlook.com",
        "Sp0t!fy#Dev2026$xx",
        "https://developer.spotify.com/",
        long,
    );

    let path = dir.join("ui-test.kdbx");
    let mut f = std::fs::File::create(&path).unwrap();
    db.save(&mut f, DatabaseKey::new().with_password(PASSWORD))
        .unwrap();
    path
}

/// Classic layout (the original interface), which most tests drive.
fn harness() -> Harness<'static, VaultApp> {
    harness_with(crate::settings::Layout::Classic, egui::vec2(1280.0, 800.0))
}

/// Floating Modular layout at a typical desktop size.
fn harness_floating() -> Harness<'static, VaultApp> {
    harness_with(crate::settings::Layout::Floating, egui::vec2(1440.0, 860.0))
}

fn harness_with(layout: crate::settings::Layout, size: egui::Vec2) -> Harness<'static, VaultApp> {
    isolate_settings();
    let mut h = Harness::builder()
        .with_size(size)
        .build_eframe(move |cc| VaultApp::with_layout(cc, layout));
    h.run_steps(2);
    h
}

/// Steps the UI until `cond` holds, for work that finishes on a background thread.
fn wait_for(
    h: &mut Harness<'_, VaultApp>,
    what: &str,
    cond: impl Fn(&Harness<'_, VaultApp>) -> bool,
) {
    let start = Instant::now();
    while !cond(h) {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
        h.step();
    }
}

fn exists(h: &Harness<'_, VaultApp>, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

/// An entry row in the list (selectable rows are buttons; the details heading is a label).
fn in_list(h: &Harness<'_, VaultApp>, title: &str) -> bool {
    h.query_all_by_role_and_label(Role::Button, title)
        .next()
        .is_some()
}

fn exists_contains(h: &Harness<'_, VaultApp>, part: &str) -> bool {
    h.query_by_label_contains(part).is_some()
}

fn type_into(h: &mut Harness<'_, VaultApp>, role: Role, index: usize, text: &str) {
    h.get_all_by_role(role)
        .nth(index)
        .expect("text field")
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(1);
    h.get_all_by_role(role).nth(index).unwrap().type_text(text);
    h.run_steps(2);
}

fn unlock(h: &mut Harness<'_, VaultApp>, db: &Path) {
    type_into(h, Role::TextInput, 0, &db.display().to_string());
    type_into(h, Role::PasswordInput, 0, PASSWORD);
    h.get_by_label("Unlock vault").click();
    // Both layouts show a "Lock" button once the vault is open.
    wait_for(h, "unlock", |h| {
        exists(h, "Lock") && !exists(h, "Unlock vault")
    });
    h.run_steps(2);
}

fn click(h: &mut Harness<'_, VaultApp>, label: &str) {
    match h.query_all_by_role_and_label(Role::Button, label).next() {
        Some(button) => button.click(),
        None => h.get_by_label(label).click(),
    }
    h.run_steps(3);
}

fn click_contains(h: &mut Harness<'_, VaultApp>, part: &str) {
    h.get_by_label_contains(part).click();
    h.run_steps(3);
}

/// Clicks the first match: categories come before groups, so this picks the category row.
fn click_first_contains(h: &mut Harness<'_, VaultApp>, part: &str) {
    h.get_all_by_label_contains(part)
        .next()
        .expect("node")
        .click();
    h.run_steps(3);
}

fn search(h: &mut Harness<'_, VaultApp>, text: &str) {
    type_into(h, Role::TextInput, 0, text);
}

fn reopen(db: &Path) -> Vault {
    Vault::open(db, creds()).unwrap()
}

#[test]
fn wrong_password_shows_error_and_stays_locked() {
    let dir = temp_dir("wrongpw");
    let db = make_db(&dir);
    let mut h = harness();
    type_into(&mut h, Role::TextInput, 0, &db.display().to_string());
    type_into(&mut h, Role::PasswordInput, 0, "nope");
    h.get_by_label("Unlock vault").click();
    wait_for(&mut h, "error", |h| {
        exists_contains(h, "wrong master password")
    });
    assert!(!exists(&h, "New entry"));
}

#[test]
fn ranked_search_and_options() {
    let dir = temp_dir("search");
    let db = make_db(&dir);
    let mut h = harness();
    unlock(&mut h, &db);

    search(&mut h, "outlook");
    assert!(in_list(&h, "Outlook"), "title match");
    assert!(
        !in_list(&h, "Spotify Developer"),
        "a login that only uses an @outlook.com email is hidden by default"
    );

    assert!(
        !in_list(&h, "Gmail"),
        "notes-only mention must not match by default"
    );
    assert!(
        !in_list(&h, "Lookout Mobile Security"),
        "'look' inside 'lookout' must not match"
    );
    click(&mut h, "emails");
    assert!(
        in_list(&h, "Spotify Developer"),
        "shown with the emails option"
    );
    let title_y = h
        .get_by_role_and_label(Role::Button, "Outlook")
        .rect()
        .top();
    let user_y = h
        .get_by_role_and_label(Role::Button, "Spotify Developer")
        .rect()
        .top();
    assert!(
        title_y < user_y,
        "title match ranks above email-domain match"
    );
    click(&mut h, "emails");

    click(&mut h, "notes");
    assert!(in_list(&h, "Gmail"), "notes match once notes are included");

    search(&mut h, "work nothing");
    assert!(exists_contains(&h, "No matches"));
}

#[test]
fn views_archive_restore_and_previous_passwords() {
    let dir = temp_dir("views");
    let db = make_db(&dir);
    let mut h = harness();
    unlock(&mut h, &db);

    click(&mut h, "All entries");
    click(&mut h, "Credit union");
    assert!(
        exists_contains(&h, "Previous passwords (2)"),
        "history shown as previous passwords"
    );

    assert!(
        exists_contains(&h, "Needs attention"),
        "health view present"
    );
    click_contains(&mut h, "Needs attention");
    assert!(
        in_list(&h, "Gmail") && in_list(&h, "Chase"),
        "reused passwords flagged"
    );
    assert!(
        !in_list(&h, "Outlook"),
        "strong unique password not flagged"
    );

    click(&mut h, "All entries");
    click(&mut h, "Chase");
    click(&mut h, "Archive");
    assert!(!in_list(&h, "Chase"), "archived entry leaves All entries");
    click_contains(&mut h, "Archived  (1)");
    assert!(in_list(&h, "Chase"), "listed under Archived");
    search(&mut h, "chase");
    assert!(!in_list(&h, "Chase"), "archived entries hidden from search");
    click(&mut h, "archived");
    assert!(in_list(&h, "Chase"), "found with the archived option");

    click(&mut h, "Chase");
    click(&mut h, "Restore");
    click(&mut h, "Save");
    let v = reopen(&db);
    let chase =
        v.db.iter_all_entries()
            .find(|e| e.get_title() == Some("Chase"))
            .unwrap();
    assert!(!organize::is_archived(&chase), "restore saved");
}

#[test]
fn edit_new_delete_save_and_lock() {
    let dir = temp_dir("edit");
    let db = make_db(&dir);
    let original = vault_core::backups::sha256_hex(&std::fs::read(&db).unwrap());
    let mut h = harness();
    unlock(&mut h, &db);

    // Edit in the editor window: title change creates a history version.
    click(&mut h, "All entries");
    click(&mut h, "Outlook");
    click(&mut h, "Edit");
    assert!(exists(&h, "Save entry"), "editor window open");
    type_into(&mut h, Role::TextInput, 1, "Outlook Mail");
    click(&mut h, "Save entry");
    assert!(!exists(&h, "Save entry"), "editor closed");
    assert!(in_list(&h, "Outlook Mail"));

    // Esc cancels an edit without changes.
    click(&mut h, "Outlook Mail");
    click(&mut h, "Edit");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(!exists(&h, "Save entry"), "Esc closes the editor");

    // New entry with a generated password, in the current group.
    click_contains(&mut h, "Email  (");
    click(&mut h, "New entry");
    type_into(&mut h, Role::TextInput, 1, "Proton");
    click(&mut h, "Save entry");
    assert!(in_list(&h, "Proton"));

    // Delete moves to the recycle bin.
    click(&mut h, "Gmail");
    click(&mut h, "Delete");
    assert!(exists(&h, "Delete entry"), "confirmation shown");
    h.get_all_by_label("Delete").last().unwrap().click();
    h.run_steps(3);
    assert!(!in_list(&h, "Gmail"));

    click(&mut h, "Save");
    let v = reopen(&db);
    let titles: Vec<String> =
        v.db.iter_all_entries()
            .map(|e| e.get_title().unwrap_or("").to_owned())
            .collect();
    assert!(titles.contains(&"Outlook Mail".to_owned()) && !titles.contains(&"Outlook".to_owned()));
    let outlook =
        v.db.iter_all_entries()
            .find(|e| e.get_title() == Some("Outlook Mail"))
            .unwrap();
    assert_eq!(
        outlook.history.as_ref().map(|h| h.get_entries().len()),
        Some(1),
        "edit kept one history version"
    );
    let proton =
        v.db.iter_all_entries()
            .find(|e| e.get_title() == Some("Proton"))
            .unwrap();
    assert!(
        proton.get_password().unwrap().chars().count() >= 20,
        "new entry got a generated password"
    );
    let gmail =
        v.db.iter_all_entries()
            .find(|e| e.get_title() == Some("Gmail"))
            .unwrap();
    assert!(
        organize::in_recycle_bin(&gmail),
        "deleted entry is in the recycle bin"
    );
    // The version the save replaced is in the backup store, not in a .bak beside the database.
    assert!(
        !PathBuf::from(format!("{}.bak", db.display())).exists(),
        "no .bak beside it"
    );
    let (folders, _) = crate::settings::backup_config().existing_folders(
        &db,
        &vault_core::backups::vault_id_of(&v.db),
        &mut Default::default(),
    );
    let kept: Vec<_> = folders.iter().flat_map(|f| f.list().unwrap()).collect();
    assert!(
        kept.iter()
            .any(|b| b.sha256.as_deref() == Some(original.as_str())),
        "the previous version is in the store"
    );

    click(&mut h, "Lock");
    assert!(exists(&h, "Unlock vault"), "back to the unlock screen");
    assert!(
        !in_list(&h, "Outlook Mail"),
        "no entries visible when locked"
    );
}

#[test]
fn unsaved_changes_prompt_on_lock() {
    let dir = temp_dir("lockprompt");
    let db = make_db(&dir);
    let mut h = harness();
    unlock(&mut h, &db);
    click(&mut h, "All entries");
    click(&mut h, "Chase");
    click(&mut h, "Archive");
    click(&mut h, "Lock");
    assert!(exists(&h, "Unsaved changes"));
    click(&mut h, "Save and lock");
    assert!(exists(&h, "Unlock vault"));
    let v = reopen(&db);
    assert!(
        v.db.iter_all_entries()
            .any(|e| e.get_title() == Some("Chase") && organize::is_archived(&e))
    );
}

#[test]
fn external_change_is_not_overwritten_silently() {
    let dir = temp_dir("external");
    let db = make_db(&dir);
    let mut h = harness();
    unlock(&mut h, &db);
    click(&mut h, "All entries");
    click(&mut h, "Chase");
    click(&mut h, "Archive");

    // Another device saves the file meanwhile.
    std::thread::sleep(Duration::from_millis(30));
    let mut other = reopen(&db);
    other
        .db
        .root_mut()
        .add_entry()
        .set_unprotected(fields::TITLE, "Added elsewhere");
    other.mark_dirty();
    other.save_overwriting().unwrap();

    click(&mut h, "Save");
    assert!(exists(&h, "File changed on disk"), "conflict prompt shown");
    click(&mut h, "Cancel");
    let v = reopen(&db);
    assert!(
        v.db.iter_all_entries()
            .any(|e| e.get_title() == Some("Added elsewhere")),
        "other device's change kept"
    );
}

/// The vault file is cut off on disk (a failed sync). Save asks about the change, "Save anyway"
/// says the file can't be read, and "Replace it" writes what is open here.
#[test]
fn save_anyway_over_an_unreadable_file_asks_then_replaces_it() {
    let dir = temp_dir("unreadable");
    let db = make_db(&dir);
    let mut h = harness();
    unlock(&mut h, &db);
    click(&mut h, "All entries");
    click(&mut h, "Chase");
    click(&mut h, "Archive");

    std::fs::write(&db, b"").unwrap();
    click(&mut h, "Save");
    assert!(exists(&h, "File changed on disk"));
    click(&mut h, "Save anyway");
    assert!(exists(&h, "The file on disk can't be read"));
    assert_eq!(
        std::fs::read(&db).unwrap(),
        b"",
        "not before it is confirmed"
    );
    click(&mut h, "Replace it");
    let v = reopen(&db);
    assert!(
        v.db.iter_all_entries()
            .any(|e| e.get_title() == Some("Chase") && organize::is_archived(&e))
    );
}

#[test]
fn details_panel_does_not_grow_with_long_notes() {
    let dir = temp_dir("panel");
    let db = make_db(&dir);
    let mut h = harness();
    unlock(&mut h, &db);
    click(&mut h, "All entries");
    click(&mut h, "Spotify Developer");
    h.run_steps(5);
    let x1 = h.get_by_label("Edit").rect().left();
    h.run_steps(30);
    let x2 = h.get_by_label("Edit").rect().left();
    assert!(
        (x1 - x2).abs() < 1.0,
        "details panel moved from {x1} to {x2}: it is growing"
    );
    assert!(x2 > 1280.0 - 600.0, "details panel wider than its cap");

    // Same for the editor's password row.
    click(&mut h, "Edit");
    h.run_steps(5);
    let g1 = h.get_by_label("Generate").rect().right();
    h.run_steps(30);
    let g2 = h.get_by_label("Generate").rect().right();
    assert!(
        (g1 - g2).abs() < 1.0,
        "editor password row grew from {g1} to {g2}"
    );
}

#[test]
fn protect_notes_and_add_protected_field() {
    let dir = temp_dir("protect");
    let db = make_db(&dir);
    let mut h = harness();
    unlock(&mut h, &db);
    click(&mut h, "All entries");
    click(&mut h, "Spotify Developer");
    assert!(
        exists_contains(&h, "Client ID d48e"),
        "notes visible before protecting"
    );
    click(&mut h, "Edit");

    click(&mut h, "Add field");
    let name_idx = h.get_all_by_role(Role::TextInput).count() - 1;
    type_into(&mut h, Role::TextInput, name_idx, "Client secret");
    let value_idx = h.get_all_by_role(Role::PasswordInput).count() - 1;
    type_into(&mut h, Role::PasswordInput, value_idx, "9711cc09a485");
    click(&mut h, "Protect notes");
    click(&mut h, "Save entry");
    assert!(!exists(&h, "Save entry"), "editor closed");

    assert!(exists(&h, "Notes (protected)"));
    assert!(
        !exists_contains(&h, "Client ID d48e"),
        "protected notes hidden in details"
    );
    assert!(exists(&h, "Client secret (protected)"));
    assert!(
        !exists(&h, "9711cc09a485"),
        "protected field hidden in details"
    );
    h.get_all_by_label("Show").last().unwrap().click();
    h.run_steps(3);
    assert!(
        exists(&h, "9711cc09a485"),
        "Show reveals the protected field"
    );

    click(&mut h, "Save");
    let v = reopen(&db);
    let e =
        v.db.iter_all_entries()
            .find(|e| e.get_title() == Some("Spotify Developer"))
            .unwrap();
    assert!(
        e.fields[fields::NOTES].is_protected(),
        "notes saved as protected"
    );
    assert!(
        e.fields["Client secret"].is_protected(),
        "field saved as protected"
    );
    assert_eq!(e.get("Client secret"), Some("9711cc09a485"));
}

#[test]
fn smart_categories_and_manual_override() {
    let dir = temp_dir("categories");
    let db = make_db(&dir);
    let mut h = harness();
    unlock(&mut h, &db);

    // Financial area groups banking entries from any folder; email stays out.
    click_contains(&mut h, "Financial  (");
    assert!(
        in_list(&h, "Chase") && in_list(&h, "Credit union"),
        "banking entries under Financial"
    );
    assert!(!in_list(&h, "Outlook"), "email is not financial");
    click_first_contains(&mut h, "Banking  (");
    assert!(
        in_list(&h, "Chase") && !in_list(&h, "Lookout Mobile Security"),
        "security app is not banking"
    );

    // Personal > Email & accounts, then override Outlook by hand.
    click_contains(&mut h, "Email & accounts  (");
    assert!(in_list(&h, "Outlook") && in_list(&h, "Gmail"));
    click(&mut h, "Outlook");
    assert!(exists(&h, "from website"));
    // The category picker is the only combo box on the main screen.
    h.get_by_role(Role::ComboBox).click();
    h.run_steps(3);
    h.get_by_label("Shopping").click_accesskit();
    h.run_steps(5);
    assert!(exists(&h, "set by you"), "manual category shown as such");

    click(&mut h, "Save");
    let v = reopen(&db);
    let outlook =
        v.db.iter_all_entries()
            .find(|e| e.get_title() == Some("Outlook"))
            .unwrap();
    assert_eq!(
        categorize::manual_category(&outlook),
        Some(categorize::Category::Shopping),
        "override saved in the entry"
    );
}

#[test]
fn floating_locked_screen_shows_no_vault_data() {
    let dir = temp_dir("float-locked");
    let db = make_db(&dir);
    let mut h = harness_floating();
    assert!(
        exists(&h, "Unlock vault"),
        "unlock form usable on the first frames"
    );
    assert!(
        exists(&h, "Close") && exists(&h, "Minimise"),
        "compact card has its own window buttons"
    );
    assert!(!exists(&h, "Maximise"), "no maximise while locked");
    for secret_bearing in ["Outlook", "Chase", "Vault", "Favorites", "Copy Password"] {
        assert!(
            !exists(&h, secret_bearing),
            "{secret_bearing} must not appear while locked"
        );
    }
    unlock(&mut h, &db);
    assert!(
        in_list(&h, "Outlook"),
        "entries appear only after unlocking"
    );
}

#[test]
fn floating_layout_end_to_end() {
    let dir = temp_dir("float");
    let db = make_db(&dir);
    let mut h = harness_floating();
    unlock(&mut h, &db);

    for nav in [
        "Vault",
        "Favorites",
        "Tags",
        "Generator",
        "Watchtower",
        "Settings",
    ] {
        assert!(exists(&h, nav), "navigation item {nav}");
    }
    // Select an entry: details, quick actions and the security panel use real data.
    click(&mut h, "Credit union");
    assert!(
        exists(&h, "Copy Username") && exists(&h, "Copy Password") && exists(&h, "Launch Site")
    );
    assert!(
        exists(&h, "Password Strength"),
        "security overview for the selected entry"
    );
    assert!(
        exists_contains(&h, "previous password"),
        "history count shown"
    );
    assert!(
        exists(&h, "No 2FA secret stored for this entry."),
        "no fake 2FA code"
    );
    assert!(
        !exists_contains(&h, "breach"),
        "no breach claims without a breach check"
    );

    // Star it: appears under Favorites and persists as a KeePass tag.
    h.get_by_label("Add to favourites").click();
    h.run_steps(3);
    click(&mut h, "Favorites");
    assert!(in_list(&h, "Credit union"));
    assert!(!in_list(&h, "Outlook"));

    // Watchtower lists the reused passwords.
    click(&mut h, "Watchtower");
    assert!(in_list(&h, "Gmail") && in_list(&h, "Chase"));

    // Generator page.
    click(&mut h, "Generator");
    assert!(exists(&h, "Password Generator") && exists(&h, "Regenerate"));

    // Save from the vault card and check the file.
    click(&mut h, "Save");
    let v = reopen(&db);
    let cu =
        v.db.iter_all_entries()
            .find(|e| e.get_title() == Some("Credit union"))
            .unwrap();
    assert!(organize::is_favorite(&cu), "favourite saved");

    // Switch to the Classic layout from Settings.
    click(&mut h, "Settings");
    click(&mut h, "Classic");
    assert!(exists(&h, "New entry"), "classic toolbar shown");
    // And back again from Classic's Settings menu.
    click(&mut h, "Settings");
    click(&mut h, "Floating Modular");
    assert!(exists(&h, "Watchtower"), "back in the Floating layout");
}

#[test]
fn floating_search_and_tags() {
    let dir = temp_dir("float-search");
    let db = make_db(&dir);
    let mut h = harness_floating();
    unlock(&mut h, &db);
    type_into(&mut h, Role::TextInput, 0, "outlook");
    assert!(in_list(&h, "Outlook") && !in_list(&h, "Gmail"));
    assert!(
        !in_list(&h, "Spotify Developer"),
        "email-domain-only match hidden"
    );
    assert!(
        exists_contains(&h, "only match because"),
        "hidden matches are explained"
    );
    click(&mut h, "Show");
    assert!(in_list(&h, "Spotify Developer"), "Show reveals them");

    h.get_all_by_role(Role::TextInput).next().unwrap().focus();
    h.run_steps(1);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(in_list(&h, "Gmail"), "Esc clears the search");
    click(&mut h, "Chase");
    click(&mut h, "Add tag");
    let idx = h.get_all_by_role(Role::TextInput).count() - 1;
    h.get_all_by_role(Role::TextInput)
        .nth(idx)
        .unwrap()
        .type_text("money");
    h.run_steps(1);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(exists(&h, "money"), "tag chip added");
    click(&mut h, "money");
    assert!(in_list(&h, "Chase"), "tag chip filters the Tags page");
}

/// A database whose key derivation takes a noticeable moment, to see the unlock in progress.
fn make_slow_db(dir: &Path) -> PathBuf {
    let path = dir.join("slow.kdbx");
    let mut db = Database::new();
    db.config.kdf_config = KdfConfig::Aes { rounds: 3_000_000 };
    db.root_mut()
        .add_entry()
        .set_unprotected(fields::TITLE, "Slow entry");
    let mut f = std::fs::File::create(&path).unwrap();
    db.save(&mut f, DatabaseKey::new().with_password(PASSWORD))
        .unwrap();
    path
}

#[test]
fn unlock_shows_real_progress_status() {
    let dir = temp_dir("slow");
    let db = make_slow_db(&dir);
    let mut h = harness_floating();
    type_into(&mut h, Role::TextInput, 0, &db.display().to_string());
    type_into(&mut h, Role::PasswordInput, 0, PASSWORD);
    h.get_by_label("Unlock vault").click();
    h.run_steps(3);
    assert!(
        exists_contains(&h, "Stretching your key  ·  AES-KDF · 3,000,000 rounds"),
        "status names the database's real key-derivation settings"
    );
    assert!(!exists(&h, "Deriving the key…"), "old spinner text gone");
    wait_for(&mut h, "unlock", |h| {
        exists(h, "Lock") && !exists(h, "Unlock vault")
    });
}

/// A database whose header asks for more key derivation than the budget allows stays locked
/// until the user ticks the override for exactly those settings, in both layouts. (101 Argon2
/// passes are over the budget, yet over 64 KiB they are quick.)
#[test]
fn heavy_kdf_unlocks_only_after_explicit_approval() {
    for floating in [false, true] {
        let dir = temp_dir("heavy-kdf");
        let db = dir.join("heavy.kdbx");
        let mut database = Database::new();
        if let KdfConfig::Argon2 {
            iterations,
            memory,
            parallelism,
            ..
        } = &mut database.config.kdf_config
        {
            (*iterations, *memory, *parallelism) = (101, 64 * 1024, 1);
        }
        database
            .save(
                &mut std::fs::File::create(&db).unwrap(),
                DatabaseKey::new().with_password(PASSWORD),
            )
            .unwrap();

        let mut h = if floating {
            harness_floating()
        } else {
            harness()
        };
        type_into(&mut h, Role::TextInput, 0, &db.display().to_string());
        type_into(&mut h, Role::PasswordInput, 0, PASSWORD);
        for _ in 0..2 {
            h.get_by_label("Unlock vault").click();
            h.run_steps(3);
            assert!(exists_contains(&h, "unusually heavy key derivation"));
            assert!(exists(&h, "Unlock vault"), "still locked without approval");
        }
        click(&mut h, crate::app::HEAVY_KDF_OVERRIDE);
        h.get_by_label("Unlock vault").click();
        wait_for(&mut h, "approved unlock", |h| {
            exists(h, "Lock") && !exists(h, "Unlock vault")
        });
        std::fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn unlock_card_switches_layout() {
    let mut h = harness_floating();
    click(&mut h, "Switch to Classic layout");
    assert!(
        exists(&h, "Switch to Floating layout"),
        "now Classic, offering the way back"
    );
    click(&mut h, "Switch to Floating layout");
    assert!(exists(&h, "Switch to Classic layout"));
}

/// Writes a two-key `.axvault` (phone + YubiKey) beside `db`.
fn write_two_key_ring(db: &Path) {
    use vault_core::envelope::{self, Envelope, KeyRing, RingKey};
    let key = [7u8; 32];
    let seal = |id: u8| Envelope::seal("localhost", &[id], [1; 32], &[id; 32], &key);
    let ring = KeyRing {
        keys: vec![
            RingKey {
                label: "Phone passkey".into(),
                envelope: seal(1),
            },
            RingKey {
                label: "Security key (USB)".into(),
                envelope: seal(2),
            },
        ],
    };
    std::fs::write(envelope::path_for(db), ring.serialize()).unwrap();
}

/// Unlocks with the password alone even though a passkey file exists (no Windows dialog in tests).
fn unlock_password_only(h: &mut Harness<'_, VaultApp>, db: &Path) {
    type_into(h, Role::TextInput, 0, &db.display().to_string());
    type_into(h, Role::PasswordInput, 0, PASSWORD);
    assert!(
        exists(h, "Passkey / security key"),
        "passkey file found, so that option is offered"
    );
    h.get_by_label("None").click();
    h.run_steps(2);
    h.get_by_label("Unlock vault").click();
    wait_for(h, "unlock", |h| {
        exists(h, "Lock") && !exists(h, "Unlock vault")
    });
    h.run_steps(2);
}

#[test]
fn settings_lists_unlock_keys() {
    let dir = temp_dir("keys");
    let db = make_db(&dir);
    let mut h = harness_floating();
    unlock(&mut h, &db);
    click(&mut h, "Settings");
    assert!(exists(&h, "Unlock keys"));
    assert!(exists(&h, "Passkey unlock: not set up"));
    assert!(exists(&h, "Set up passkey unlock…"));
    assert!(
        exists_contains(&h, "FIDO2 security key (YubiKey)"),
        "setup mentions security keys, not just phones"
    );

    write_two_key_ring(&db);
    let mut h = harness_floating();
    unlock_password_only(&mut h, &db);
    click(&mut h, "Settings");
    assert!(exists(&h, "Phone passkey") && exists(&h, "Security key (USB)"));
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, "Remove")
            .count(),
        2
    );
}

/// Column tests share the saved column choice, so they take turns.
static COLUMNS_FILE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn classic_columns_can_be_shown_and_hidden() {
    let _turn = COLUMNS_FILE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = temp_dir("columns");
    let db = make_db(&dir);
    let mut h = harness();
    unlock(&mut h, &db);
    click(&mut h, "All entries");
    assert!(exists(&h, "Website") && !exists(&h, "Group"));

    click(&mut h, "Settings");
    h.get_by_label("Columns ⏵").hover();
    h.run_steps(3);
    h.get_by_label("Columns ⏵").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::CheckBox, "Group").click();
    h.run_steps(3);
    assert!(exists(&h, "Group"), "Group column added");
    assert!(exists(&h, "Banking"), "shows each entry's group");
    h.get_by_role_and_label(Role::CheckBox, "Website").click();
    h.run_steps(3);
    assert!(!exists(&h, "Website") || h.query_all_by_label("Website").count() == 1);
    assert!(
        crate::settings::columns().contains(&crate::columns::Col::Group)
            && !crate::settings::columns().contains(&crate::columns::Col::Website),
        "choice is remembered"
    );
    click(&mut h, "Reset to default columns");
    assert_eq!(crate::settings::columns(), crate::columns::defaults());
}

#[test]
fn classic_columns_sort_and_right_click() {
    let _turn = COLUMNS_FILE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = temp_dir("colsort");
    let db = make_db(&dir);
    let mut h = harness();
    unlock(&mut h, &db);
    click(&mut h, "All entries");
    let y = |h: &Harness<'_, VaultApp>, title: &str| {
        h.get_by_role_and_label(Role::Button, title).rect().top()
    };
    assert!(
        y(&h, "Gmail") < y(&h, "Lookout Mobile Security"),
        "A-Z by title"
    );

    // Website: lookout.com sorts before mail.google.com; a second click reverses.
    h.get_by_role_and_label(Role::Button, "Website").click();
    h.run_steps(3);
    assert!(
        y(&h, "Lookout Mobile Security") < y(&h, "Gmail"),
        "sorted by website"
    );
    h.get_by_role_and_label(Role::Button, "Website").click();
    h.run_steps(3);
    assert!(
        y(&h, "Gmail") < y(&h, "Lookout Mobile Security"),
        "reversed"
    );
    assert_eq!(
        crate::settings::table_sort(),
        (Some(crate::columns::Col::Website), true),
        "sort choice is remembered"
    );

    // Right-click a header to add a column.
    h.get_by_role_and_label(Role::Button, "Username")
        .click_secondary();
    h.run_steps(3);
    h.get_by_role_and_label(Role::CheckBox, "Tags").click();
    h.run_steps(3);
    assert!(crate::settings::columns().contains(&crate::columns::Col::Tags));
    h.get_by_role_and_label(Role::CheckBox, "Tags").click();
    h.run_steps(3);
    assert!(!crate::settings::columns().contains(&crate::columns::Col::Tags));
    crate::settings::set_table_sort(
        Some(&crate::columns::Col::Custom("Account #".into())),
        false,
    );
    assert_eq!(
        crate::settings::table_sort(),
        (Some(crate::columns::Col::Custom("Account #".into())), false),
        "custom field names with spaces survive"
    );
    crate::settings::set_table_sort(None, false);
}

/// Whether the button `label` is greyed out.
fn disabled(h: &Harness<'_, VaultApp>, label: &str) -> bool {
    use egui_kittest::kittest::NodeT;
    h.get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

/// The Backups section has listed the backups (that runs on a worker).
fn wait_for_backups(h: &mut Harness<'_, VaultApp>) {
    wait_for(h, "the backups listing", |h| {
        exists(h, "Back up now") && !exists(h, "Looking for backups…")
    });
}

/// The backups of the vault at `db` in the app's (test) backup folders.
fn stored_backups(db: &Path) -> Vec<vault_core::backups::Backup> {
    let (folders, _) = crate::settings::backup_config().existing_folders(
        db,
        &vault_core::backups::vault_id_of(&reopen(db).db),
        &mut Default::default(),
    );
    folders.iter().flat_map(|f| f.list().unwrap()).collect()
}

/// "Back up now" pins a copy of the vault as saved, so it waits for unsaved changes ("Save
/// first"). Both layouts show the same Backups section: inline on the Floating settings page,
/// as a dialog from the Classic Settings menu.
#[test]
fn back_up_now_disabled_when_dirty() {
    let dir = temp_dir("backupnow");
    let db = make_db(&dir);
    // Tall enough for the whole Settings page.
    let mut h = harness_with(
        crate::settings::Layout::Floating,
        egui::vec2(1440.0, 1700.0),
    );
    unlock(&mut h, &db);
    click(&mut h, "Credit union");
    h.get_by_label("Add to favourites").click();
    h.run_steps(3);
    click(&mut h, "Settings");
    wait_for_backups(&mut h);
    assert!(disabled(&h, "Back up now"), "unsaved changes");
    assert!(exists(&h, "Save first"));
    assert!(disabled(&h, "Restore…"));
    assert!(exists_contains(&h, "Keeps the newest 20 saves"));
    assert!(exists_contains(&h, "phone apps can't see them"));

    click(&mut h, "Save");
    wait_for_backups(&mut h);
    assert!(!disabled(&h, "Back up now") && !exists(&h, "Save first"));
    click(&mut h, "Back up now");
    assert!(exists_contains(&h, "Backed up as"));
    wait_for(&mut h, "the new backup listed", |h| {
        exists_contains(h, "(Back up now)")
    });
    let saved = vault_core::backups::sha256_hex(&std::fs::read(&db).unwrap());
    assert!(
        stored_backups(&db).iter().any(|b| {
            b.name.kind() == Some(vault_core::backups::Kind::Manual)
                && b.sha256.as_deref() == Some(saved.as_str())
        }),
        "a pinned copy of the saved vault"
    );

    let mut h = harness();
    unlock(&mut h, &db);
    click(&mut h, "All entries");
    click(&mut h, "Chase");
    click(&mut h, "Archive");
    click(&mut h, "Settings");
    click(&mut h, "Backups…");
    wait_for_backups(&mut h);
    assert!(disabled(&h, "Back up now") && exists(&h, "Save first"));
    assert!(!disabled(&h, "Delete all backups"));
    click(&mut h, "Close");
    assert!(!exists(&h, "Back up now"), "the dialog closed");
}

/// A file in a backup folder opens as a backup: a banner says so, Save and Ctrl+S do nothing,
/// locking doesn't save or ask, and its path is never remembered.
#[test]
fn backup_opened_as_vault_is_read_only_and_not_remembered() {
    let dir = temp_dir("readonly");
    let db = make_db(&dir);
    let folder = crate::settings::backup_config()
        .default_root
        .join(dir.file_name().unwrap());
    std::fs::create_dir_all(&folder).unwrap();
    // A vault's backup folder, as the store makes it.
    std::fs::write(
        folder.join(vault_core::backups::VAULT_FILE),
        br#"{"v":1,"id":"0123456789abcdef0123456789abcdef","stem":"ui-test"}"#,
    )
    .unwrap();
    let backup = folder.join("ui-test 2026-10-14 09-05-00 #000001.kdbx");
    std::fs::copy(&db, &backup).unwrap();
    let before = std::fs::read(&backup).unwrap();
    let path = backup.display().to_string();

    let mut h = harness();
    type_into(&mut h, Role::TextInput, 0, &path);
    assert!(exists(&h, crate::app::BACKUP_UNLOCK_HINT));
    unlock(&mut h, &backup);
    assert!(exists_contains(
        &h,
        "Backup from 2026-10-14 09:05. Changes here aren't saved."
    ));
    click(&mut h, "All entries");
    click(&mut h, "Chase");
    click(&mut h, "Archive");
    assert!(exists(&h, "Lock"), "still open");
    assert!(disabled(&h, "Save"), "Save is off");
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    h.run_steps(3);
    click(&mut h, "Lock");
    assert!(
        exists(&h, "Unlock vault") && !exists(&h, "Unsaved changes"),
        "locked without asking to save"
    );
    assert!(exists_contains(&h, "Locked the backup without saving"));
    assert!(exists(&h, crate::app::BACKUP_UNLOCK_HINT));
    assert_eq!(std::fs::read(&backup).unwrap(), before, "never saved");
    assert_eq!(
        std::fs::read_dir(&folder).unwrap().count(),
        2,
        "nothing written beside it and vault.json"
    );
    assert_ne!(crate::settings::last_database(), Some(path.clone()));
    assert!(crate::settings::unlock_ms(&path).is_none());
}

#[test]
fn every_ui_character_has_a_glyph() {
    let h = harness();
    let src = concat!(
        include_str!("app.rs"),
        include_str!("session.rs"),
        include_str!("session/floating.rs"),
        include_str!("session/backups.rs"),
        include_str!("chrome.rs")
    );
    let chars: std::collections::BTreeSet<char> = src.chars().filter(|c| !c.is_ascii()).collect();
    let font = egui::FontId::proportional(14.0);
    let missing: Vec<String> = chars
        .into_iter()
        .filter(|&c| !h.ctx.fonts_mut(|f| f.has_glyph(&font, c)))
        .map(|c| format!("U+{:04X} {c}", c as u32))
        .collect();
    assert!(
        missing.is_empty(),
        "characters the font cannot draw (shown as empty boxes): {missing:?}"
    );
}

/// Renders the real UI to PNGs for visual review (not run by default):
/// `cargo test -p axentra-vault render_previews -- --ignored`
#[test]
#[ignore]
fn render_previews() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("preview");
    std::fs::create_dir_all(&out).unwrap();
    let mut h = harness();
    // Mid-animation and settled frames of the unlock screen.
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(900) {
        h.step();
        std::thread::sleep(Duration::from_millis(16));
    }
    h.render()
        .unwrap()
        .save(out.join("unlock-0.9s.png"))
        .unwrap();
    while start.elapsed() < Duration::from_millis(2600) {
        h.step();
        std::thread::sleep(Duration::from_millis(16));
    }
    h.render()
        .unwrap()
        .save(out.join("unlock-final.png"))
        .unwrap();

    let dir = temp_dir("preview");
    let db = make_db(&dir);
    unlock(&mut h, &db);
    click(&mut h, "All entries");
    click(&mut h, "Spotify Developer");
    h.run_steps(5);
    h.render()
        .unwrap()
        .save(out.join("vault-main.png"))
        .unwrap();

    // Floating Modular layout: locked (after the entrance) and open.
    // The locked window is just the unlock card.
    let mut f = harness_with(crate::settings::Layout::Floating, crate::app::COMPACT_SIZE);
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(2600) {
        f.step();
        std::thread::sleep(Duration::from_millis(16));
    }
    f.render()
        .unwrap()
        .save(out.join("floating-locked.png"))
        .unwrap();

    // Unlocking in progress (slow key derivation, so the animation is mid-flight).
    let slow = make_slow_db(&dir);
    type_into(&mut f, Role::TextInput, 0, &slow.display().to_string());
    type_into(&mut f, Role::PasswordInput, 0, PASSWORD);
    f.get_by_label("Unlock vault").click();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(250) {
        f.step();
        std::thread::sleep(Duration::from_millis(16));
    }
    f.render()
        .unwrap()
        .save(out.join("floating-unlocking.png"))
        .unwrap();
    while start.elapsed() < Duration::from_millis(5000)
        && exists(&f, "Unlock vault")
        && !exists(&f, "Unlocked")
    {
        f.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    f.step();
    std::thread::sleep(Duration::from_millis(150));
    f.step();
    f.render()
        .unwrap()
        .save(out.join("floating-opening.png"))
        .unwrap();
    let mut f = harness_floating();
    unlock(&mut f, &db);
    click(&mut f, "Credit union");
    f.run_steps(5);
    f.render()
        .unwrap()
        .save(out.join("floating-main.png"))
        .unwrap();

    // Settings with two unlock keys (phone + YubiKey).
    write_two_key_ring(&db);
    let mut f = harness_floating();
    unlock_password_only(&mut f, &db);
    click(&mut f, "Settings");
    f.run_steps(5);
    f.render()
        .unwrap()
        .save(out.join("floating-settings-keys.png"))
        .unwrap();
}

/// Screenshots for the public product page, with neutral demo data (not run by default):
/// `cargo test -p axentra-vault store_screenshots -- --ignored`
#[test]
#[ignore]
fn store_screenshots() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("preview")
        .join("store");
    std::fs::create_dir_all(&out).unwrap();
    let dir = temp_dir("store");
    let path = dir.join("Personal.kdbx");
    let mut db = Database::new();
    db.config.kdf_config = KdfConfig::Aes { rounds: 1000 };
    db.root_mut().name = "Personal".into();
    let entries: &[(&str, &str, &str, &str, &str)] = &[
        (
            "Banking",
            "Chase",
            "alex.morgan",
            "Vq7#mR2!tK9$wL4p",
            "https://www.chase.com",
        ),
        (
            "Banking",
            "Credit union",
            "amorgan",
            "Hn3@xP8&cZ5!rB1e",
            "https://www.navyfederal.org",
        ),
        (
            "Banking",
            "Fidelity",
            "alex.morgan",
            "Td6$yW1#kM4^qS8u",
            "https://www.fidelity.com",
        ),
        (
            "Email",
            "Gmail",
            "alex.morgan@gmail.com",
            "Lp2!vN7@fG3#hJ6z",
            "https://mail.google.com",
        ),
        (
            "Email",
            "Outlook",
            "alex.morgan@outlook.com",
            "summer2019",
            "https://outlook.live.com",
        ),
        (
            "Shopping",
            "Amazon",
            "alex.morgan@gmail.com",
            "summer2019",
            "https://www.amazon.com",
        ),
        (
            "Shopping",
            "PayPal",
            "alex.morgan@gmail.com",
            "Rc5^bX9!mQ2$tV7k",
            "https://www.paypal.com",
        ),
        (
            "Streaming",
            "Netflix",
            "alex.morgan@gmail.com",
            "Wf8#jL3@pD6!sH1n",
            "https://www.netflix.com",
        ),
        (
            "Streaming",
            "Spotify",
            "alexmorgan",
            "Ky4$gT7^zR2#mC9v",
            "https://open.spotify.com",
        ),
        (
            "Work",
            "GitHub",
            "amorgan-dev",
            "Bz1!qF6@wN8$xP3d",
            "https://github.com",
        ),
        (
            "Work",
            "Microsoft 365",
            "alex.morgan@contoso.com",
            "Mj9@rK2#vB5!tL8q",
            "https://www.office.com",
        ),
    ];
    let mut groups = std::collections::HashMap::new();
    for (g, title, user, pw, url) in entries {
        let gid = *groups.entry(*g).or_insert_with(|| {
            let mut r = db.root_mut();
            let mut grp = r.add_group();
            grp.name = (*g).into();
            grp.id()
        });
        let mut grp = db.group_mut(gid).unwrap();
        let mut e = grp.add_entry();
        e.set_unprotected(fields::TITLE, *title);
        e.set_unprotected(fields::USERNAME, *user);
        e.set_protected(fields::PASSWORD, *pw);
        e.set_unprotected(fields::URL, *url);
        e.times.last_modification = Some(Times::now());
    }
    db.save(
        &mut std::fs::File::create(&path).unwrap(),
        DatabaseKey::new().with_password(PASSWORD),
    )
    .unwrap();

    let settle = |h: &mut Harness<'_, VaultApp>, ms: u64| {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(ms) {
            h.step();
            std::thread::sleep(Duration::from_millis(16));
        }
    };

    // Locked: the compact unlock card, with a neutral-looking path.
    let mut f = harness_with(crate::settings::Layout::Floating, crate::app::COMPACT_SIZE);
    settle(&mut f, 2600);
    type_into(
        &mut f,
        Role::TextInput,
        0,
        r"C:\Users\Alex\Documents\Personal.kdbx",
    );
    settle(&mut f, 200);
    f.render().unwrap().save(out.join("locked.png")).unwrap();

    // Floating layout with an entry open.
    let mut f = harness_floating();
    unlock(&mut f, &path);
    click(&mut f, "Chase");
    f.run_steps(5);
    f.render().unwrap().save(out.join("floating.png")).unwrap();

    // Watchtower: the reused password is flagged.
    click(&mut f, "Watchtower");
    click(&mut f, "Amazon");
    f.run_steps(5);
    f.render()
        .unwrap()
        .save(out.join("watchtower.png"))
        .unwrap();

    // Classic layout.
    let mut c = harness();
    unlock(&mut c, &path);
    click(&mut c, "All entries");
    click(&mut c, "Fidelity");
    c.run_steps(5);
    c.render().unwrap().save(out.join("classic.png")).unwrap();
}
