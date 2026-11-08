//! Round trip against files written by KeePass itself. Run through `interop/run.ps1`, which sets
//! AXV_INTEROP_DIR after generating the fixtures with KeePassLib; skipped otherwise.

use std::path::PathBuf;

use vault_core::envelope::{Envelope, KeyRing, RingKey};
use vault_core::fidelity;
use vault_core::organize;
use vault_core::vault::{Credentials, Vault};
use zeroize::Zeroizing;

const PASSWORD: &str = "fixture-password";

fn dir() -> Option<PathBuf> {
    std::env::var_os("AXV_INTEROP_DIR").map(PathBuf::from)
}

fn creds() -> Credentials {
    Credentials {
        password: Some(Zeroizing::new(PASSWORD.into())),
        key_file: Some(Zeroizing::new(vec![0x5A; 32])),
    }
}

#[test]
fn keepass_fixture_survives_save() {
    let Some(dir) = dir() else { return };
    let data = std::fs::read(dir.join("fixture.kdbx")).unwrap();

    let report = fidelity::check(&data, creds().to_key().unwrap()).unwrap();
    for (path, value, before, after) in &report.differences {
        eprintln!("DIFF {path} [{value}] before={before} after={after}");
    }
    eprintln!("fallback icons dropped: {}", report.fallback_icons_dropped);
    assert!(report.elements_before > 100, "fixture unexpectedly small");
    assert!(
        report.is_clean(),
        "{} XML differences after save",
        report.differences.len()
    );

    // Save through the app's own path (verify + atomic replace) for the KeePassLib check.
    let resaved = dir.join("rust-resaved.kdbx");
    std::fs::copy(dir.join("fixture.kdbx"), &resaved).unwrap();
    let mut v = Vault::open(&resaved, creds()).unwrap();
    // Edit the history-bearing entry the way the app does, so KeePass can check the order.
    let id =
        v.db.iter_all_entries()
            .find(|e| e.get_title() == Some("Bank <main> & \"co\""))
            .map(|e| e.id())
            .expect("fixture entry");
    assert!(organize::edit_with_history(&mut v.db, id, |e| {
        let mut form = vault_core::form::EntryForm::from_entry(e);
        *form.password = "rust-password".into();
        form.notes_protected = true;
        form.custom.push(vault_core::form::CustomField {
            name: "Client secret".into(),
            value: Zeroizing::new("from-rust".into()),
            protected: true,
        });
        form.apply(e);
    }));
    v.mark_dirty();
    // A KeePass-written file has no credential record yet: the save adds one, which KeePassLib
    // checks against its own composite key in `verify`.
    assert!(!v.generation_committed() && !v.outside_change());
    v.save().unwrap();
    let generation = v.generation().to_owned();
    drop(v);
    let reopened = Vault::open(&resaved, creds()).unwrap();
    assert_eq!(reopened.generation(), generation);
    assert!(reopened.generation_committed() && !reopened.outside_change());
}

#[test]
fn passkey_files_cross_open() {
    let Some(dir) = dir() else { return };
    let prf = [0x11u8; 32];
    let key = [0x22u8; 32];

    let from_csharp =
        Envelope::parse(&std::fs::read_to_string(dir.join("csharp.axvault")).unwrap()).unwrap();
    assert_eq!(
        *from_csharp.open(&prf).unwrap(),
        key,
        "C#-sealed envelope must open in Rust"
    );

    let cred_id: Vec<u8> = (1..=16).collect();
    let ours = Envelope::seal("localhost", &cred_id, [0x33; 32], &prf, &key);
    std::fs::write(dir.join("rust.axvault"), ours.serialize()).unwrap();

    // A multi-key file (phone + security key), as written when a second key is added.
    let second = Envelope::seal("localhost", &[0x44; 20], [0x33; 32], &[0x55; 32], &key);
    let ring = KeyRing {
        keys: vec![
            RingKey {
                label: "Phone passkey".into(),
                envelope: ours,
            },
            RingKey {
                label: "Security key (USB)".into(),
                envelope: second,
            },
        ],
    };
    std::fs::write(dir.join("rust-ring.axvault"), ring.serialize()).unwrap();
}
