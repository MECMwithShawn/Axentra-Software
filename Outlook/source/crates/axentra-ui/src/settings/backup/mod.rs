//! Settings › Accounts › Backup: encrypted backups of the account setups
//! and the data that exists only on this device, saved into the OneDrive
//! folder that Axentra OneDrive syncs (see `axentra_core::backup`).
//!
//! The page acts immediately (no Save / Discard). The password lives in
//! the keyring ([`vault`]); [`host::BackupHost`] runs the monthly backup.

mod host;
mod password;
mod restore;
mod vault;

use axentra_core::backup::{self, BackupFile, Counts, dest::DestKind};
use chrono::{DateTime, Utc};
use dioxus::prelude::*;

pub use host::{BackupHost, preview_mode};
pub use vault::use_memory_keyring;

use super::controls::{Note, Row, Section};
use super::pages::PageId;
use crate::kit::{Button, ConfirmDialog, EmptyState, Icon, IconButton, Intent, MessageBar, ProgressBar, Select, Switch, TextField, Variant, dom};
use crate::state::{store, use_app};
use host::{REV, RUN, RunState};
use password::{AskPasswordDialog, NewPasswordFields};
use restore::RestoreDialog;
use vault::KeyringState;

/// "Sun 10/18/2026 4:52 PM".
pub(crate) fn when(t: DateTime<Utc>, h24: bool) -> String {
    crate::fmt::full_time(t, h24)
}

/// "12 messages · 1 draft · 40 events", leaving out zeros.
pub(crate) fn counts_text(c: &Counts) -> String {
    c.items()
        .into_iter()
        .map(|(what, n)| {
            let what = if n == 1 {
                match what {
                    "categories" => "category".to_string(),
                    w => w.strip_suffix('s').unwrap_or(w).to_string(),
                }
            } else {
                what.to_string()
            };
            format!("{n} {what}")
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Choices for "Keep the last".
const KEEP: [u32; 7] = [1, 2, 3, 6, 12, 24, 36];

/// The status card: (intent, icon, title, detail).
fn status(
    running: bool,
    last: Option<&backup::LastBackup>,
    newest: Option<&BackupFile>,
    failure: Option<&backup::LastFailure>,
    h24: bool,
) -> (Intent, &'static str, String, String) {
    if running {
        return (Intent::Info, "arrow_sync", "Backing up…".into(), "This can take a minute. You can keep working.".into());
    }
    if let Some(f) = failure {
        return (Intent::Error, "error_circle", "The last backup didn't finish".into(), format!("{} ({})", f.message, when(f.at, h24)));
    }
    match (last, newest) {
        (Some(l), _) => (
            Intent::Success,
            "checkmark_circle",
            format!("Last backup: {}", when(l.at, h24)),
            if std::path::Path::new(&l.path).exists() {
                format!("{} · {}", crate::fmt::file_size(l.size as i64), l.path)
            } else {
                format!("The file has since been moved or deleted ({}).", l.path)
            },
        ),
        // Made before this install, or with another folder setting.
        (None, Some(f)) => (
            Intent::Success,
            "checkmark_circle",
            format!("Newest backup in this folder: {}", when(f.created_at, h24)),
            format!("{} · {}", crate::fmt::file_size(f.size as i64), f.name),
        ),
        (None, None) => (
            Intent::Info,
            "shield",
            "No backups yet".into(),
            "Back up now to save your account setups and everything that's only on this device.".into(),
        ),
    }
}

fn tone(intent: Intent) -> &'static str {
    match intent {
        Intent::Info => "ax-tone-info",
        Intent::Success => "ax-tone-success",
        Intent::Warning => "ax-tone-warning",
        Intent::Error => "ax-tone-error",
    }
}

#[component]
pub fn BackupPage() -> Element {
    let app = use_app();
    let _ = REV.read();
    let _ = app.accounts_rev();
    let settings = app.settings.read().clone();
    let h24 = settings.use_24h_time;
    let target = host::destination(&settings);
    let mut key_rev = use_signal(|| 0u64);
    let keyring = use_resource(move || async move {
        let _ = key_rev();
        vault::state().await
    });
    let key_state = *keyring.read();
    let remembered = key_state == Some(KeyringState::Remembered);
    let can_remember = matches!(key_state, Some(KeyringState::Remembered | KeyringState::NotSet | KeyringState::Locked));

    let mut asking = use_signal(|| false);
    let mut restoring = use_signal(|| None::<BackupFile>);
    let mut deleting = use_signal(|| None::<BackupFile>);
    let mut forgetting = use_signal(|| false);
    let mut changing = use_signal(|| false);
    let mut new_pw = use_signal(String::new);
    let mut new_confirm = use_signal(String::new);
    let mut folder = use_signal(|| settings.backup_dir.clone().unwrap_or_default());
    let mut folder_error = use_signal(|| None::<String>);

    let run = RUN.read().clone();
    let running = matches!(run, RunState::Running { .. });
    let last = backup::last_backup(store());
    let failure = backup::last_failure(store()).filter(|f| last.as_ref().is_none_or(|l| f.at > l.at));
    let files = backup::list_backups(&target.dir);
    let newest = files.as_ref().ok().and_then(|f| f.first().cloned());
    let (intent, icon, title, detail) = status(running, last.as_ref(), newest.as_ref(), failure.as_ref(), h24);
    let auto_on = settings.backup_auto && remembered;
    let next = if !remembered {
        match key_state {
            Some(KeyringState::Unavailable) => "Automatic backups are off: there's no keyring to remember the password.".to_string(),
            Some(KeyringState::Locked) => "Automatic backups wait until your keyring is unlocked.".to_string(),
            None => String::new(),
            _ => "Automatic backups start once you save a backup password above.".to_string(),
        }
    } else if !settings.backup_auto {
        "Automatic backups are off.".to_string()
    } else {
        let due = backup::next_due(host::last_success(), Utc::now());
        if due <= Utc::now() { "Next automatic backup: within the hour.".to_string() } else { format!("Next automatic backup: {}", when(due, h24)) }
    };

    let back_up_now = move |_| {
        if host::is_running() {
            return;
        }
        spawn(async move {
            match vault::get(true).await {
                Ok(Some(pw)) if !pw.is_empty() => {
                    host::start(app, pw, true);
                }
                _ => asking.set(true),
            }
        });
    };
    let save_password = move || {
        if backup::password_problem(&new_pw.peek(), &new_confirm.peek()).is_some() {
            return;
        }
        let secret = axentra_sync::Secret::new(new_pw.peek().clone());
        spawn(async move {
            match vault::set(secret).await {
                Ok(()) => {
                    new_pw.set(String::new());
                    new_confirm.set(String::new());
                    changing.set(false);
                    key_rev += 1;
                    app.toast("Backup password saved in your keyring");
                    // The form goes away; keep keyboard focus on the page.
                    dom::focus("#ax-backup-change-password");
                }
                Err(e) => {
                    app.error(format!("Couldn't save the password. {}", e.message()));
                }
            }
        });
    };
    let mut apply_folder = move || {
        let text = folder.peek().trim().to_string();
        if text.is_empty() {
            app.update_settings(|s| s.backup_dir = None);
            folder_error.set(None);
            return;
        }
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        // A full path or one in the home folder; a bare relative path is
        // too easy to get wrong.
        let full = text.starts_with('/') || text == "~" || text.starts_with("~/");
        match backup::dest::expand(&text, home.as_deref()).filter(|_| full) {
            Some(p) => {
                folder_error.set(None);
                app.update_settings(|s| s.backup_dir = Some(text.clone()));
                dom::announce(&format!("Backups will be saved in {}", backup::dest::backup_dir(&p).display()), false);
            }
            _ => folder_error.set(Some("Enter a full folder path, for example ~/Backups or /media/usb.".into())),
        }
    };

    let in_onedrive = target.in_onedrive;
    let waiting_for_mount = target.waiting_for_mount();
    let dir_text = target.dir.display().to_string();
    let custom = settings.backup_dir.is_some();
    rsx! {
        Section { title: "Back up your accounts",
            description: "Saves your account setups, settings and the mail, events, contacts and tasks that exist only on this device to one encrypted file. Mail that's on a server isn't included; passwords and sign-ins never are.",
            div { class: "ax-backup-status {tone(intent)}", role: "status", "aria-live": "polite",
                Icon { name: icon, size: 24 }
                div { class: "ax-backup-status-main",
                    span { class: "ax-backup-status-title", "{title}" }
                    span { class: "ax-backup-status-detail ax-selectable", "{detail}" }
                    if auto_on && !running { span { class: "ax-backup-status-detail", "{next}" } }
                    if running { ProgressBar { value: None, label: "Backing up" } }
                }
            }
            div { class: "ax-backup-actions",
                Button {
                    id: PageId::anchor("Back up now"),
                    label: if running { "Backing up…" } else { "Back up now" },
                    icon: "save",
                    variant: Variant::Primary,
                    disabled: running,
                    title: if running { Some("A backup is running".to_string()) } else { None },
                    onclick: back_up_now,
                }
            }
            match run.clone() {
                RunState::Finished(Ok(done)) => rsx! {
                    MessageBar {
                        intent: Intent::Success,
                        title: "Backup complete.",
                        text: format!(
                            "Saved {} ({}){}{}",
                            done.file,
                            crate::fmt::file_size(done.size as i64),
                            if done.in_onedrive { " to OneDrive — synced by Axentra OneDrive.".to_string() } else { format!(" in {}.", done.folder) },
                            match done.removed {
                                0 => String::new(),
                                1 => " Removed 1 older backup.".to_string(),
                                n => format!(" Removed {n} older backups."),
                            }
                        ),
                        on_dismiss: move |_| *RUN.write() = RunState::Idle,
                    }
                },
                RunState::Finished(Err(m)) => rsx! {
                    MessageBar { intent: Intent::Error, title: "Backup didn't finish.", text: m, on_dismiss: move |_| *RUN.write() = RunState::Idle }
                },
                _ => rsx! {},
            }
        }

        Section { title: "Where backups are saved",
            div { class: "ax-backup-dest", id: PageId::anchor("OneDrive backup folder"),
                div { class: "ax-backup-path ax-selectable", "aria-label": "Backup folder", "{dir_text}" }
                if waiting_for_mount {
                    span { class: "ax-backup-badge ax-backup-badge-warning", Icon { name: "warning", size: 16 } "Axentra OneDrive isn't running, so backups wait until it is" }
                } else if in_onedrive {
                    span { class: "ax-backup-badge ax-onedrive", Icon { name: "checkmark_circle", size: 16 } "Saved to OneDrive — synced by Axentra OneDrive" }
                } else {
                    span { class: "ax-backup-badge", Icon { name: "cloud_off", size: 16 } "Not in your OneDrive folder, so it isn't copied to the cloud" }
                }
                if target.kind == DestKind::Documents {
                    Note { text: "No OneDrive folder was found, so backups go to Documents. Set up Axentra OneDrive to keep them in the cloud." }
                }
                if target.kind == DestKind::Override {
                    Note { text: "Set by AXENTRA_BACKUP_DIR (for testing). Unset it to choose a folder here." }
                } else {
                    div { class: "ax-backup-folder-form",
                        TextField {
                            label: "Use a different folder",
                            value: folder(),
                            placeholder: "Leave empty to use your OneDrive folder",
                            hint: folder_error().unwrap_or_else(|| "Backups go into Axentra/Outlook Backups inside this folder.".into()),
                            class: if folder_error().is_some() { "ax-backup-field-error" } else { "" },
                            oninput: move |v| {
                                folder.set(v);
                                folder_error.set(None);
                            },
                            onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { apply_folder() },
                        }
                        Button {
                            label: "Use this folder",
                            disabled: folder().trim() == settings.backup_dir.clone().unwrap_or_default().trim(),
                            title: "Save backups in this folder from now on",
                            onclick: move |_| apply_folder(),
                        }
                        if custom {
                            Button {
                                label: "Use OneDrive folder",
                                title: "Go back to the default folder",
                                onclick: move |_| {
                                    folder.set(String::new());
                                    app.update_settings(|s| s.backup_dir = None);
                                },
                            }
                        }
                    }
                }
            }
        }

        Section { title: "Backup password",
            description: "Backups are encrypted with this password. Keep it safe: without it, a backup can't be opened.",
            match key_state {
                None => rsx! { crate::kit::Spinner { label: "Checking your keyring…", size: 16 } },
                Some(KeyringState::Remembered) if !changing() => rsx! {
                    div { class: "ax-backup-remembered", Icon { name: "lock_closed", size: 16 } "Remembered in your keyring" }
                    div { class: "ax-backup-actions",
                        Button {
                            id: "ax-backup-change-password",
                            label: "Change password",
                            icon: "edit",
                            onclick: move |_| {
                                changing.set(true);
                                // Once the form has rendered, start in its first field.
                                spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                                    dom::focus(".ax-backup-pw input");
                                });
                            },
                        }
                        Button { label: "Forget password", icon: "delete", onclick: move |_| forgetting.set(true) }
                    }
                },
                Some(KeyringState::Unavailable) => rsx! {
                    Note { icon: "warning", text: "No keyring is available, so the password can't be remembered. Back up now asks for it each time, and automatic backups are off." }
                },
                Some(KeyringState::Locked) if !changing() => rsx! {
                    Note { icon: "lock_closed", text: "Your keyring is locked." }
                    div { class: "ax-backup-actions",
                        Button {
                            label: "Unlock keyring",
                            onclick: move |_| {
                                spawn(async move {
                                    let _ = vault::get(true).await;
                                    key_rev += 1;
                                });
                            },
                        }
                    }
                },
                _ => rsx! {
                    if changing() {
                        Note { text: "Backups you already made keep the password they were made with." }
                    }
                    NewPasswordFields { password: new_pw, confirm: new_confirm, on_submit: move |_| save_password() }
                    div { class: "ax-backup-actions",
                        Button {
                            label: "Save password",
                            variant: Variant::Primary,
                            disabled: backup::password_problem(&new_pw(), &new_confirm()).is_some(),
                            title: "Remember this password in your keyring",
                            onclick: move |_| save_password(),
                        }
                        if changing() {
                            Button {
                                label: "Cancel",
                                onclick: move |_| {
                                    changing.set(false);
                                    new_pw.set(String::new());
                                    new_confirm.set(String::new());
                                },
                            }
                        }
                    }
                },
            }
        }

        Section { title: "Schedule",
            Switch {
                label: "Back up automatically every month",
                checked: auto_on,
                disabled: !remembered,
                title: if remembered { "Runs in the background about every 30 days" } else { "Save a backup password in your keyring first" },
                onchange: move |v| app.update_settings(|s| s.backup_auto = v),
            }
            if !next.is_empty() { Note { text: next.clone() } }
            Row { label: "Keep the last", description: "After each backup, older backups made on this device are deleted. Other files, and backups from other computers, are never touched.",
                Select {
                    value: settings.backup_keep.to_string(),
                    aria_label: "Number of backups to keep",
                    options: KEEP.iter().map(|n| (n.to_string(), if *n == 1 { "1 backup".to_string() } else { format!("{n} backups") })).collect::<Vec<_>>(),
                    onchange: move |v: String| if let Ok(n) = v.parse::<u32>() { app.update_settings(|s| s.backup_keep = n.max(1)) },
                }
            }
        }

        Section { title: "Your backups",
            match files {
                Err(e) => rsx! { MessageBar { intent: Intent::Error, text: e.to_string() } },
                Ok(list) if list.is_empty() => rsx! {
                    div { class: "ax-backup-empty",
                        EmptyState { icon: "history", title: "No backups in this folder yet", subtitle: "Backups you make appear here, newest first." }
                    }
                },
                Ok(list) => rsx! {
                    ul { class: "ax-backup-list", id: PageId::anchor("Restore from a backup"), "aria-label": "Backups, newest first",
                        for (i, f) in list.into_iter().enumerate() {
                            BackupRow {
                                key: "{f.name}",
                                file: f.clone(),
                                latest: i == 0,
                                h24,
                                on_restore: move |f| restoring.set(Some(f)),
                                on_delete: move |f| deleting.set(Some(f)),
                            }
                        }
                    }
                },
            }
        }

        if asking() {
            AskPasswordDialog {
                can_remember,
                on_close: move |_| asking.set(false),
                on_submit: move |(pw, remember): (axentra_sync::Secret, bool)| {
                    asking.set(false);
                    spawn(async move {
                        if remember {
                            match vault::set(pw.clone()).await {
                                Ok(()) => key_rev += 1,
                                Err(e) => {
                                    app.error(format!("The password wasn't remembered. {}", e.message()));
                                }
                            }
                        }
                        host::start(app, pw, true);
                    });
                },
            }
        }
        if let Some(f) = restoring() {
            RestoreDialog { file: f, on_close: move |_| restoring.set(None) }
        }
        if let Some(f) = deleting() {
            ConfirmDialog {
                title: "Delete this backup?",
                message: format!("The backup from {} will be permanently deleted. This can't be undone.", when(f.created_at, h24)),
                confirm_label: "Delete",
                danger: true,
                on_cancel: move |_| deleting.set(None),
                on_confirm: move |_| {
                    deleting.set(None);
                    match backup::delete_backup(&f.path) {
                        Ok(()) => {
                            app.toast("Backup deleted");
                        }
                        Err(e) => {
                            app.error(e.to_string());
                        }
                    }
                    *REV.write() += 1;
                },
            }
        }
        if forgetting() {
            ConfirmDialog {
                title: "Forget the backup password?",
                message: "It's removed from your keyring. Automatic backups stop until you save a password again, and Back up now will ask for one. Existing backups still open with it.",
                confirm_label: "Forget",
                on_cancel: move |_| forgetting.set(false),
                on_confirm: move |_| {
                    forgetting.set(false);
                    spawn(async move {
                        match vault::forget().await {
                            Ok(()) => {
                                app.toast("Backup password forgotten");
                            }
                            Err(e) => {
                                app.error(format!("Couldn't remove the password. {}", e.message()));
                            }
                        }
                        key_rev += 1;
                    });
                },
            }
        }
    }
}

#[component]
fn BackupRow(file: BackupFile, latest: bool, h24: bool, on_restore: EventHandler<BackupFile>, on_delete: EventHandler<BackupFile>) -> Element {
    let date = when(file.created_at, h24);
    let size = crate::fmt::file_size(file.size as i64);
    let f1 = file.clone();
    let f2 = file.clone();
    rsx! {
        li { class: "ax-backup-row",
            Icon { name: "document", size: 20 }
            div { class: "ax-backup-row-main",
                span { class: "ax-backup-row-date",
                    span { class: "ax-ellipsis", "{date}" }
                    if latest { span { class: "ax-backup-pill", "Latest" } }
                }
                span { class: "ax-backup-row-meta ax-ellipsis", title: "{file.name}", "{size} · {file.name}" }
            }
            div { class: "ax-backup-row-actions",
                Button { label: "Restore…", small: true, title: "Restore from the backup of {date}", onclick: move |_| on_restore.call(f1.clone()) }
                IconButton { icon: "delete", title: "Delete the backup of {date}", small: true, onclick: move |_| on_delete.call(f2.clone()) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_naturally() {
        let c = Counts { messages: 12, drafts: 1, events: 40, categories: 1, contact_lists: 1, folders: 9, ..Default::default() };
        assert_eq!(counts_text(&c), "12 messages · 1 draft · 40 events · 1 contact list · 1 category");
        assert_eq!(counts_text(&Counts::default()), "");
    }

    #[test]
    fn status_prefers_running_then_failure_then_last() {
        let at = Utc::now();
        let here = std::env::temp_dir().display().to_string();
        let last = backup::LastBackup { at, size: 2048, path: here.clone() };
        let fail = backup::LastFailure { at, message: "Disk full.".into(), notified_on: None };
        let file = BackupFile { path: "/b/y.axobk".into(), name: "y.axobk".into(), created_at: at, size: 1 };
        assert_eq!(status(true, Some(&last), None, Some(&fail), false).2, "Backing up…");
        assert_eq!(status(false, Some(&last), None, Some(&fail), false).0, Intent::Error);
        let (i, _, title, detail) = status(false, Some(&last), Some(&file), None, false);
        assert_eq!(i, Intent::Success);
        assert!(title.starts_with("Last backup: ") && detail == format!("2 KB · {here}"));
        let gone = backup::LastBackup { path: "/no/such/backup.axobk".into(), ..last.clone() };
        assert!(status(false, Some(&gone), None, None, false).3.starts_with("The file has since been moved or deleted"));
        assert!(status(false, None, Some(&file), None, false).2.starts_with("Newest backup in this folder: "));
        assert_eq!(status(false, None, None, None, false).2, "No backups yet");
    }
}
