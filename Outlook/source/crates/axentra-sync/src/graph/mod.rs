//! Microsoft Graph provider (§4).

pub mod calendar;
pub mod client;
pub mod contacts;
pub mod mail;
pub mod map;
pub mod photo;
pub mod todo;

use serde_json::Value;

pub use client::{Graph, TokenSource};

/// `GET /me` identity (§4.5): `mail` may be empty for some personal
/// accounts, so fall back to `userPrincipalName`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Me {
    pub id: String,
    pub display_name: String,
    pub email: String,
}

pub fn me_from(v: &Value) -> Me {
    let mail = v["mail"].as_str().filter(|m| !m.is_empty());
    let upn = v["userPrincipalName"].as_str().unwrap_or_default();
    Me {
        id: v["id"].as_str().unwrap_or_default().to_string(),
        display_name: v["displayName"].as_str().unwrap_or_default().to_string(),
        email: mail.unwrap_or(upn).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn me_falls_back_to_upn() {
        let me = me_from(&serde_json::json!({"id": "1", "displayName": "Ana", "mail": null, "userPrincipalName": "ana@outlook.com"}));
        assert_eq!(me.email, "ana@outlook.com");
        let me = me_from(&serde_json::json!({"id": "1", "displayName": "Ana", "mail": "ana@contoso.com", "userPrincipalName": "x"}));
        assert_eq!(me.email, "ana@contoso.com");
    }
}
