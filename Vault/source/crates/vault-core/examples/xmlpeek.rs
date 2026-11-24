// Debug helper: print selected lines of a database's inner XML before and after a save.
// usage: xmlpeek <db> <password> <32-byte keyfile hex> <substring>...
use keepass::{Database, DatabaseKey};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&a[1]).unwrap();
    let kf = hex::decode(&a[3]).unwrap();
    let key = || {
        DatabaseKey::new()
            .with_password(&a[2])
            .with_keyfile(&mut &kf[..])
            .unwrap()
    };
    let db = Database::parse(&data, key()).unwrap();
    let mut saved = Vec::new();
    db.save(&mut saved, key()).unwrap();
    for (label, bytes) in [("BEFORE", &data), ("AFTER", &saved)] {
        let xml = String::from_utf8(Database::get_xml(&mut &bytes[..], key()).unwrap()).unwrap();
        println!("===== {label}");
        for line in xml
            .split('>')
            .map(|s| s.trim())
            .filter(|l| a[4..].iter().any(|p| l.contains(p.as_str())))
        {
            println!("{line}>");
        }
    }
}
