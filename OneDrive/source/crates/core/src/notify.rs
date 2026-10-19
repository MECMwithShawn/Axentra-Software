//! Desktop notifications (org.freedesktop.Notifications), used where the
//! Windows client pops a toast: mass deletes, conflicts, sync problems.

use std::collections::HashMap;

use tokio::sync::OnceCell;
use zbus::zvariant::Value;

pub struct Notifier {
    enabled: bool,
    conn: OnceCell<Option<zbus::Connection>>,
}

impl Notifier {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            conn: OnceCell::new(),
        }
    }

    pub async fn send(&self, summary: &str, body: &str) {
        if !self.enabled {
            return;
        }
        let conn = self
            .conn
            .get_or_init(|| async { zbus::Connection::session().await.ok() })
            .await;
        let Some(conn) = conn else { return };
        let hints: HashMap<&str, Value<'_>> = HashMap::new();
        let result = conn
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &(
                    "Axentra OneDrive",
                    0u32,
                    "axentra-onedrive",
                    summary,
                    body,
                    Vec::<&str>::new(),
                    hints,
                    -1i32,
                ),
            )
            .await;
        if let Err(e) = result {
            tracing::debug!("notification failed: {e}");
        }
    }
}
