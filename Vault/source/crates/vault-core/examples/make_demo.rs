// Writes a demo database for trying the app: `cargo run --example make_demo -- <path>`.
// Password: demo. Contains a search trap for "outlook", reused/weak/old passwords, history,
// an archived account and a recycle bin.
use keepass::db::{Times, fields};
use keepass::{Database, DatabaseKey};
use vault_core::organize;

fn main() {
    let path = std::env::args().nth(1).expect("usage: make_demo <path>");
    let mut db = Database::new();
    db.meta.database_name = Some("Axentra Vault demo".into());
    db.meta.recyclebin_enabled = Some(true);
    let years_ago = Times::now() - chrono::Duration::days(800);

    let root = db.root().id();
    let mut group = |name: &str| {
        let mut r = db.root_mut();
        let mut g = r.add_group();
        g.name = name.into();
        g.id()
    };
    let email_id = group("Email");
    let bank_id = group("Banking");
    let bin_id = group("Recycle Bin");
    db.meta.recyclebin_uuid = Some(bin_id.uuid());

    let add = |db: &mut Database, g, title: &str, user: &str, pw: &str, url: &str, notes: &str| {
        let mut grp = db.group_mut(g).unwrap();
        let mut e = grp.add_entry();
        e.set_unprotected(fields::TITLE, title);
        e.set_unprotected(fields::USERNAME, user);
        e.set_protected(fields::PASSWORD, pw);
        e.set_unprotected(fields::URL, url);
        e.set_unprotected(fields::NOTES, notes);
        let now = Times::now();
        e.times.creation = Some(now);
        e.times.last_modification = Some(now);
        e.id()
    };

    add(
        &mut db,
        email_id,
        "Outlook",
        "shawn@outlook.com",
        "Kp7#vQ2!mZ9@rT4$wX8&",
        "https://outlook.live.com/mail",
        "",
    );
    add(
        &mut db,
        email_id,
        "Gmail",
        "shawn@gmail.com",
        "summer2019",
        "https://mail.google.com",
        "Recovery address is my Outlook account",
    );
    add(
        &mut db,
        bank_id,
        "Chase",
        "shawnb",
        "summer2019",
        "https://chase.com",
        "Statements also go to outlook",
    );
    add(
        &mut db,
        root,
        "Lookout Mobile Security",
        "shawn",
        "Lm4$Tr8!Qw2#Zx6@Pv9",
        "https://lookout.com",
        "",
    );
    let netflix = add(
        &mut db,
        root,
        "Netflix",
        "shawn@outlook.com",
        "Nf1$aB7!",
        "https://netflix.com",
        "",
    );
    let old = add(
        &mut db,
        bank_id,
        "Credit union",
        "shawn.b",
        "first-password-2019",
        "https://cu.example",
        "",
    );
    for pw in ["second-password-2021", "Cu9#dL4!pW7@kQ2$mZ5&"] {
        organize::edit_with_history(&mut db, old, |e| e.set_protected(fields::PASSWORD, pw));
    }
    let myspace = add(
        &mut db,
        root,
        "MySpace",
        "shawn85",
        "tom-is-my-friend",
        "https://myspace.com",
        "Old account",
    );
    organize::set_archived(&mut db.entry_mut(myspace).unwrap(), true);
    add(
        &mut db,
        bin_id,
        "Deleted forum account",
        "shawn",
        "x",
        "https://forum.example",
        "",
    );
    db.entry_mut(netflix).unwrap().times.last_modification = Some(years_ago);

    let mut f = std::fs::File::create(&path).unwrap();
    db.save(&mut f, DatabaseKey::new().with_password("demo"))
        .unwrap();
    println!("wrote {path} (password: demo)");
}
