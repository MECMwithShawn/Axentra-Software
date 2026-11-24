//! The editable form of an entry: standard fields plus custom fields, each of which can be
//! protected (hidden in the UI, zeroized on drop, and encrypted in the file's inner stream, the same
//! "Protect value" flag KeePass uses). Loading and applying keep every protection flag, so an
//! edit never silently unprotects a value KeePass had protected.

use std::collections::BTreeMap;

use keepass::db::{Entry, Value, fields};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomField {
    pub name: String,
    pub value: Zeroizing<String>,
    pub protected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryForm {
    pub title: String,
    pub username: String,
    pub password: Zeroizing<String>,
    pub url: String,
    pub notes: String,
    pub notes_protected: bool,
    pub custom: Vec<CustomField>,
    title_protected: bool,
    username_protected: bool,
    url_protected: bool,
}

fn is_custom(name: &str) -> bool {
    !fields::KNOWN_FIELDS.contains(&name)
}

fn get(e: &Entry, name: &str) -> (String, bool) {
    e.fields
        .get(name)
        .map(|v| (v.get().clone(), v.is_protected()))
        .unwrap_or_default()
}

fn value(text: &str, protected: bool) -> Value<String> {
    if protected {
        Value::protected(text.to_owned())
    } else {
        Value::unprotected(text.to_owned())
    }
}

impl EntryForm {
    /// A blank form for a new entry with the given (usually generated) password.
    pub fn new_with_password(password: Zeroizing<String>) -> Self {
        Self {
            password,
            title: String::new(),
            username: String::new(),
            url: String::new(),
            notes: String::new(),
            notes_protected: false,
            custom: Vec::new(),
            title_protected: false,
            username_protected: false,
            url_protected: false,
        }
    }

    pub fn from_entry(e: &Entry) -> Self {
        let (title, title_protected) = get(e, fields::TITLE);
        let (username, username_protected) = get(e, fields::USERNAME);
        let (url, url_protected) = get(e, fields::URL);
        let (notes, notes_protected) = get(e, fields::NOTES);
        let mut custom: Vec<CustomField> = e
            .fields
            .iter()
            .filter(|(k, _)| is_custom(k))
            .map(|(k, v)| CustomField {
                name: k.clone(),
                value: Zeroizing::new(v.get().clone()),
                protected: v.is_protected(),
            })
            .collect();
        custom.sort_by_key(|f| f.name.to_lowercase());
        EntryForm {
            title,
            username,
            password: Zeroizing::new(e.get_password().unwrap_or("").to_owned()),
            url,
            notes,
            notes_protected,
            custom,
            title_protected,
            username_protected,
            url_protected,
        }
    }

    /// A problem that must be fixed before saving, if any.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for f in &self.custom {
            let name = f.name.trim();
            if name.is_empty() {
                return Err("Every custom field needs a name.".into());
            }
            if !is_custom(name) {
                return Err(format!(
                    "\"{name}\" is a standard field name; choose another."
                ));
            }
            if !seen.insert(name.to_lowercase()) {
                return Err(format!("There are two fields named \"{name}\"."));
            }
        }
        Ok(())
    }

    fn custom_map(&self) -> BTreeMap<String, (Zeroizing<String>, bool)> {
        self.custom
            .iter()
            .map(|f| (f.name.trim().to_owned(), (f.value.clone(), f.protected)))
            .collect()
    }

    /// Whether applying this form would change the entry (values or protection flags).
    pub fn differs_from(&self, e: &Entry) -> bool {
        let current = EntryForm::from_entry(e);
        let standard = (
            &self.title,
            &self.username,
            self.password.as_str(),
            &self.url,
            &self.notes,
            self.notes_protected,
        ) != (
            &current.title,
            &current.username,
            current.password.as_str(),
            &current.url,
            &current.notes,
            current.notes_protected,
        );
        standard || self.custom_map() != current.custom_map()
    }

    /// Writes the form into the entry. Passwords are always protected; title, username and URL
    /// keep whatever protection they had; notes and custom fields follow the form.
    pub fn apply(&self, e: &mut Entry) {
        e.set(fields::TITLE, value(&self.title, self.title_protected));
        e.set(
            fields::USERNAME,
            value(&self.username, self.username_protected),
        );
        e.set(fields::PASSWORD, value(&self.password, true));
        e.set(fields::URL, value(&self.url, self.url_protected));
        e.set(fields::NOTES, value(&self.notes, self.notes_protected));

        let wanted = self.custom_map();
        let stale: Vec<String> = e
            .fields
            .keys()
            .filter(|k| is_custom(k) && !wanted.contains_key(*k))
            .cloned()
            .collect();
        for k in stale {
            e.fields.remove(&k);
        }
        for (name, (text, protected)) in wanted {
            e.set(name, value(&text, protected));
        }
    }
}

impl Drop for EntryForm {
    fn drop(&mut self) {
        self.title.zeroize();
        self.username.zeroize();
        self.url.zeroize();
        self.notes.zeroize();
        for field in &mut self.custom {
            field.name.zeroize();
        }
        // password and custom values are Zeroizing<String>.
    }
}

#[cfg(test)]
mod tests {
    use keepass::Database;

    use super::*;

    fn entry(db: &mut Database) -> keepass::db::EntryId {
        db.root_mut()
            .add_entry()
            .edit(|e| {
                e.set_unprotected(fields::TITLE, "Spotify Developer");
                e.set_protected(fields::PASSWORD, "pw");
                e.set_unprotected(fields::NOTES, "App status: development");
                e.set_protected("Client secret", "9711cc09");
                e.set_unprotected("Client ID", "d48e7898");
            })
            .id()
    }

    #[test]
    fn round_trip_keeps_protection_flags() {
        let mut db = Database::new();
        let id = entry(&mut db);
        let e = db.entry(id).unwrap();
        let form = EntryForm::from_entry(&e);
        assert_eq!(form.custom.len(), 2);
        assert!(
            form.custom
                .iter()
                .any(|f| f.name == "Client secret" && f.protected)
        );
        assert!(!form.differs_from(&e), "unchanged form is not a change");
    }

    #[test]
    fn protect_notes_and_add_protected_field() {
        let mut db = Database::new();
        let id = entry(&mut db);
        let mut form = EntryForm::from_entry(&db.entry(id).unwrap());
        form.notes_protected = true;
        form.custom.push(CustomField {
            name: "API key".into(),
            value: Zeroizing::new("abc".into()),
            protected: true,
        });
        assert!(
            form.differs_from(&db.entry(id).unwrap()),
            "protection change counts as a change"
        );
        form.apply(&mut db.entry_mut(id).unwrap());

        let e = db.entry(id).unwrap();
        assert!(e.fields[fields::NOTES].is_protected());
        assert!(e.fields["API key"].is_protected());
        assert_eq!(e.get("API key"), Some("abc"));
        assert!(e.fields[fields::PASSWORD].is_protected());
    }

    #[test]
    fn removing_and_renaming_fields() {
        let mut db = Database::new();
        let id = entry(&mut db);
        let mut form = EntryForm::from_entry(&db.entry(id).unwrap());
        form.custom.retain(|f| f.name != "Client ID");
        form.custom
            .iter_mut()
            .find(|f| f.name == "Client secret")
            .unwrap()
            .name = "Secret".into();
        form.apply(&mut db.entry_mut(id).unwrap());

        let e = db.entry(id).unwrap();
        assert!(e.get("Client ID").is_none());
        assert!(e.get("Client secret").is_none());
        assert_eq!(e.get("Secret"), Some("9711cc09"));
        assert!(e.fields["Secret"].is_protected(), "rename keeps protection");
    }

    #[test]
    fn existing_protection_on_standard_fields_is_kept() {
        let mut db = Database::new();
        let id = db
            .root_mut()
            .add_entry()
            .edit(|e| {
                e.set_protected(fields::USERNAME, "secret-user");
                e.set_protected(fields::NOTES, "secret notes");
            })
            .id();
        let mut form = EntryForm::from_entry(&db.entry(id).unwrap());
        form.title = "Renamed".into();
        form.apply(&mut db.entry_mut(id).unwrap());
        let e = db.entry(id).unwrap();
        assert!(
            e.fields[fields::USERNAME].is_protected(),
            "KeePass-protected username stays protected"
        );
        assert!(
            e.fields[fields::NOTES].is_protected(),
            "protected notes stay protected"
        );
    }

    #[test]
    fn validation() {
        let mut form = EntryForm::default();
        assert!(form.validate().is_ok());
        let field = |n: &str| CustomField {
            name: n.into(),
            value: Zeroizing::new(String::new()),
            protected: false,
        };
        form.custom = vec![field(" ")];
        assert!(form.validate().is_err());
        form.custom = vec![field("Password")];
        assert!(form.validate().is_err());
        form.custom = vec![field("PIN"), field("pin")];
        assert!(form.validate().is_err());
        form.custom = vec![field("PIN"), field("Account")];
        assert!(form.validate().is_ok());
    }
}
