//! Settings > Backups, one section shared by both layouts: the Floating settings page shows it
//! inline, and the Classic Settings menu opens it as a dialog. Also here: the prompt after
//! unlock to move old copies from beside the database into the backup folder, and the banner
//! of a backup opened read-only.
//!
//! Listing and deleting backups run on a worker (a slow or unplugged backup folder can take a
//! moment to answer); "Back up now", restoring, moving old files and choosing the folder run on
//! the UI thread, like saving.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use vault_core::backups::{BackupName, Kind};
use vault_core::manage::{self, DeleteReport, MigrationReport, OldFile, Overview, RootWarning};
use vault_core::vault::VaultError;

use super::{Confirm, Session, View};
use crate::jobs::{self, Job};

/// Retention, stated as text (there are no number fields to get wrong).
pub(super) const RETENTION: &str = "Keeps the newest 20 saves, one per day for the last 30 days with saves, one per month for the last 12 months with saves, and every manual, restore, key-change and imported backup.";
pub(super) const RESTORE_WARNING: &str = "Close this vault on your other devices and apps first; restoring replaces it everywhere it syncs.";
pub(super) const OPEN_FOLDER_HINT: &str =
    "To recover a backup, use Restore. Copying a file over your vault can lock you out.";
/// Shown beside "Back up now" while it is disabled.
pub(super) const SAVE_FIRST: &str = "Save first";
const ABOUT: [&str; 3] = [
    "Backups stay on this PC (or in the folder you choose) and are not synced; phone apps can't see them.",
    "They open with the same master password and key as the vault, and don't replace your backup key file.",
    "Copies outside the backup folder (OneDrive version history, other apps' backups, phones) keep the credentials they were made with.",
];

/// The Backups section's state.
pub(super) struct Backups {
    overview: Option<Overview>,
    /// Listing, or deleting then listing, on a worker.
    job: Option<Job<Done>>,
    /// The listing is out of date (a save, a backup, a restore...); it is redone while shown.
    stale: bool,
    /// Old files beside the database, checked with each listing.
    old: Vec<OldFile>,
    /// The result of the last action taken here, and whether it went wrong.
    message: Option<(String, bool)>,
    ask: Option<Ask>,
    /// The restore picker is open.
    picking: bool,
    /// Classic: the Backups dialog is open.
    pub(super) dialog: bool,
    /// The prompt to move old files into the backup folder, with the files.
    pub(super) migration: Option<Vec<OldFile>>,
}

impl Default for Backups {
    fn default() -> Self {
        Backups {
            overview: None,
            job: None,
            stale: true,
            old: Vec::new(),
            message: None,
            ask: None,
            picking: false,
            dialog: false,
            migration: None,
        }
    }
}

/// A question the section asks before acting.
enum Ask {
    DeleteOlder(usize),
    DeleteAll(usize),
    /// Use this folder despite the warnings?
    Root(PathBuf, Vec<RootWarning>),
    /// Stop using this earlier folder?
    Forget(PathBuf),
}

/// What a worker job returns.
struct Done {
    overview: Overview,
    deleted: Option<DeleteReport>,
}

/// Why each kind of backup was taken, for the list.
fn kind_text(kind: Kind) -> &'static str {
    match kind {
        Kind::Auto => "Before a save",
        Kind::Opened => "At unlock",
        Kind::ReplacedExternal => "Replaced by Save anyway",
        Kind::Manual => "Back up now",
        Kind::BeforeRestore => "Before a restore",
        Kind::BeforeKeyChange => "Before a key change",
        Kind::Imported => "Imported",
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The OneDrive folders the environment names, for the sync-folder warning.
fn onedrive_roots() -> Vec<PathBuf> {
    ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(std::env::var_os)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Shows `folder` in File Explorer.
fn open_in_explorer(folder: &Path) -> Result<(), String> {
    let explorer = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("explorer.exe"))
        .unwrap_or_else(|| PathBuf::from("explorer.exe"));
    std::process::Command::new(explorer)
        .arg(folder)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Could not open File Explorer: {e}"))
}

/// What moving old files did, in words, and whether anything was left behind.
pub(super) fn migration_message(report: &MigrationReport) -> (String, bool) {
    let mut parts = Vec::new();
    if !report.moved.is_empty() {
        parts.push(format!(
            "Moved {} into the backup folder.",
            plural(report.moved.len(), "old copy", "old copies")
        ));
    }
    if !report.removed.is_empty() {
        parts.push(format!(
            "Deleted {}.",
            plural(
                report.removed.len(),
                "old passkey file",
                "old passkey files"
            )
        ));
    }
    for (path, why) in &report.kept {
        parts.push(format!(
            "{} was copied into the backup folder but is still beside the vault ({why}). Close it, then use Move old .bak again.",
            file_name(path)
        ));
    }
    for (path, why) in &report.failed {
        parts.push(format!("{} was left where it is ({why}).", file_name(path)));
    }
    if parts.is_empty() {
        parts.push("There were no old copies to move.".into());
    }
    let problem = !report.kept.is_empty() || !report.failed.is_empty();
    (parts.join(" "), problem)
}

impl Session {
    /// The banner of a backup opened as a vault, naming the backup by its time.
    pub(super) fn read_only_banner(&self) -> Option<String> {
        if !self.vault.is_read_only() {
            return None;
        }
        let name = file_name(self.vault.path());
        let when = BackupName::parse(&name)
            .map(|n| {
                format!(
                    "{} {}",
                    n.date,
                    n.time.get(..5).unwrap_or(&n.time).replace('-', ":")
                )
            })
            .unwrap_or(name);
        Some(format!(
            "Backup from {when}. Changes here aren't saved. To restore it, open your vault and use Settings > Backups > Restore."
        ))
    }

    /// After unlock: asks once whether to move old copies from beside the database into the
    /// backup folder, unless the owner said not to ask again for this vault.
    pub fn offer_migration(&mut self) {
        if self.vault.backup_config().is_none() {
            return;
        }
        let old = self.vault.old_files();
        if !old.is_empty() && !self.vault.migration_declined() {
            self.backups.migration = Some(old);
        }
    }

    /// The listing is out of date; it is redone the next time the section is shown.
    pub(super) fn backups_changed(&mut self) {
        self.backups.stale = true;
    }

    /// Applies a finished listing or delete (run every frame, whether or not it is shown).
    pub(super) fn poll_backups(&mut self) {
        let Some(result) = self.backups.job.as_ref().and_then(Job::poll) else {
            return;
        };
        self.backups.job = None;
        match result {
            Ok(done) => {
                if let Some(report) = done.deleted {
                    let mut text =
                        format!("Deleted {}.", plural(report.deleted, "backup", "backups"));
                    if report.left > 0 {
                        text.push_str(&format!(
                            " {} in use by another program and stay for now.",
                            plural(report.left, "is", "are")
                        ));
                    }
                    for e in &report.errors {
                        text.push_str(&format!(" {e}."));
                    }
                    let problem = report.left > 0 || !report.errors.is_empty();
                    self.backups_result(text, problem);
                }
                self.backups.overview = Some(done.overview);
            }
            Err(jobs::WorkerDied) => self.backups_result(
                "Looking through the backups stopped unexpectedly; nothing was deleted.".into(),
                true,
            ),
        }
    }

    fn backups_result(&mut self, text: String, problem: bool) {
        if problem {
            self.error = Some(text.clone());
        } else {
            self.notify(text.clone());
        }
        self.backups.message = Some((text, problem));
    }

    /// Lists the backups again on a worker, unless a job is running already.
    fn refresh_backups(&mut self, ctx: &egui::Context) {
        if self.backups.job.is_some() {
            return;
        }
        let Some(mut task) = self.vault.backups_task() else {
            return;
        };
        self.backups.stale = false;
        self.backups.old = self.vault.old_files();
        self.backups.job = Some(Job::spawn(ctx, move || Done {
            overview: task.overview(),
            deleted: None,
        }));
    }

    fn start_delete(&mut self, ctx: &egui::Context, all: bool) {
        let Some(mut task) = self.vault.backups_task() else {
            return;
        };
        self.backups.stale = false;
        self.backups.job = Some(Job::spawn(ctx, move || {
            let deleted = if all {
                task.delete_all()
            } else {
                task.delete_older()
            };
            Done {
                overview: task.overview(),
                deleted: Some(deleted),
            }
        }));
    }

    pub(super) fn back_up_now(&mut self) {
        match self.vault.back_up_now() {
            Ok(written) => {
                let mut text = format!("Backed up as {}.", file_name(written.path()));
                if let Some(why) = self.vault.take_backup_notice() {
                    text.push(' ');
                    text.push_str(&why);
                }
                self.backups_result(text, false);
            }
            Err(VaultError::ChangedOnDisk) => self.backups_result(
                "Another device or app saved this vault since you opened it. Lock and reopen it to back up that version.".into(),
                true,
            ),
            Err(e) => self.backups_result(format!("Back up failed: {e}"), true),
        }
        self.backups_changed();
    }

    /// Restores a backup through the normal save (see `Vault::restore`), then resets what the
    /// views show of the old contents.
    pub(super) fn restore_backup(&mut self, path: &Path, when: &str) {
        let result = self.vault.restore(path);
        // On ChangedOnDisk too the session holds the restored contents.
        if matches!(result, Ok(()) | Err(VaultError::ChangedOnDisk)) {
            // Groups and entries of the replaced version may be gone.
            self.view = View::All;
            self.selected = None;
            self.editor = None;
            self.icon_cache.clear();
            self.invalidate_categories();
        }
        match result {
            Ok(()) => self.backups_result(
                format!(
                    "Restored the backup from {when}. The version it replaced is kept as a backup."
                ),
                false,
            ),
            // The restored contents are in the session, unsaved, for "Save anyway".
            Err(VaultError::ChangedOnDisk) => self.confirm = Some(Confirm::Overwrite),
            Err(e @ VaultError::NeedsOlderCredentials) => self.backups_result(e.to_string(), true),
            Err(e) => self.backups_result(format!("Restore failed: {e}"), true),
        }
        self.backups.picking = false;
        self.backups_changed();
    }

    pub(super) fn migrate_old_files(&mut self) {
        match self.vault.migrate_old_files() {
            Ok(report) => {
                let (text, problem) = migration_message(&report);
                self.backups_result(text, problem);
            }
            Err(e) => self.backups_result(format!("Could not move the old copies: {e}"), true),
        }
        self.backups_changed();
    }

    /// "Change...": a folder picker, the checks, then (after any warnings are confirmed) the
    /// new folder.
    fn change_backup_root(&mut self) {
        let Some(config) = self.vault.backup_config().cloned() else {
            return;
        };
        let Some(path) = crate::dialogs::file()
            .set_title("Choose the backup folder")
            .pick_folder()
        else {
            return;
        };
        match manage::check_new_root(&path, self.vault.path(), &config, &onedrive_roots()) {
            Err(why) => self.backups_result(why, true),
            Ok(warnings) if warnings.is_empty() => self.set_backup_root(Some(path)),
            Ok(warnings) => self.backups.ask = Some(Ask::Root(path, warnings)),
        }
    }

    pub(super) fn forget_backup_root(&mut self, path: &Path) {
        match self.vault.forget_backup_root(path) {
            Ok(_) => self.backups_result(
                format!(
                    "Axentra no longer uses {}. The backups there stay where they are.",
                    path.display()
                ),
                false,
            ),
            Err(e) => self.backups_result(format!("Could not forget that folder: {e}"), true),
        }
        self.backups_changed();
    }

    fn set_backup_root(&mut self, path: Option<PathBuf>) {
        match self.vault.set_backup_root(path.as_deref()) {
            Ok(()) => {
                let now = self
                    .vault
                    .backup_config()
                    .map(|c| c.home().path.display().to_string())
                    .unwrap_or_default();
                self.backups_result(
                    format!("New backups go to {now}. Earlier backups stay where they are and are still listed."),
                    false,
                );
            }
            Err(e) => self.backups_result(format!("Could not use that folder: {e}"), true),
        }
        self.backups_changed();
    }

    /// The Backups section: where backups go, how many there are, and what can be done with
    /// them. The caller draws the heading.
    pub(super) fn backups_section(&mut self, ui: &mut egui::Ui) {
        let warn = ui.visuals().warn_fg_color;
        let error = ui.visuals().error_fg_color;
        if self.vault.is_read_only() {
            ui.label(
                RichText::new("This is a backup opened read-only. Its vault's backups are managed from the vault itself: open the vault and use Settings > Backups.")
                    .small()
                    .weak(),
            );
            return;
        }
        let Some(config) = self.vault.backup_config().cloned() else {
            ui.label(
                RichText::new("No backup folder is set up for this vault.")
                    .small()
                    .weak(),
            );
            return;
        };
        if self.backups.stale {
            self.refresh_backups(ui.ctx());
        }
        let busy = self.backups.job.is_some();
        let dirty = self.vault.is_dirty();
        let home = config.home();
        let chosen = config.root.is_some();

        // Where they go.
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Location:").small());
            ui.label(
                RichText::new(home.path.display().to_string())
                    .small()
                    .strong(),
            );
            if !chosen {
                ui.label(RichText::new("(default)").small().weak());
            }
        });
        ui.horizontal(|ui| {
            if ui
                .button("Change…")
                .on_hover_text("Keep backups in another folder, such as a USB drive")
                .clicked()
            {
                self.change_backup_root();
            }
            if chosen
                && ui
                    .button("Use default")
                    .on_hover_text(config.default_root.display().to_string())
                    .clicked()
            {
                self.set_backup_root(None);
            }
        });
        // Earlier folders stay in use until forgotten.
        for previous in &config.previous_roots {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("Earlier folder: {}", previous.path.display()))
                        .small()
                        .weak(),
                );
                if ui
                    .small_button("Forget")
                    .on_hover_text("Stop listing, cleaning up and re-encrypting the backups there")
                    .clicked()
                {
                    self.backups.ask = Some(Ask::Forget(previous.path.clone()));
                }
            });
        }

        // How many, and anything wrong.
        let overview = self.backups.overview.clone();
        match &overview {
            None if busy => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Looking for backups…").small());
                });
            }
            None => {
                ui.label(
                    RichText::new("The backups couldn't be listed.")
                        .small()
                        .color(warn),
                );
            }
            Some(o) => {
                let count = match o.newest() {
                    None => "No backups yet.".to_owned(),
                    Some(newest) => format!(
                        "{}; newest {} ({}).",
                        plural(o.backups.len(), "backup", "backups"),
                        newest.when(),
                        kind_text(newest.kind())
                    ),
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(count).small());
                    if busy {
                        ui.spinner();
                    }
                });
                if let Some(why) = &o.unavailable {
                    ui.label(
                        RichText::new(format!(
                            "The chosen folder isn't available ({why}); backups go to {} meanwhile.",
                            config.default_root.display()
                        ))
                        .small()
                        .color(warn),
                    );
                }
                for (path, why) in &o.unreachable {
                    ui.label(
                        RichText::new(format!(
                            "Backups in {} can't be read right now ({why}).",
                            path.display()
                        ))
                        .small()
                        .weak(),
                    );
                }
                if o.older() > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} older credentials.",
                            plural(o.older(), "backup uses", "backups use")
                        ))
                        .small()
                        .color(warn),
                    );
                }
            }
        }
        for line in [self.vault.backup_status(), self.vault.backup_warning()]
            .into_iter()
            .flatten()
        {
            ui.label(RichText::new(line).small().color(warn));
        }
        if let Some((text, problem)) = &self.backups.message {
            ui.label(RichText::new(text).small().color(if *problem {
                error
            } else {
                ui.visuals().text_color()
            }));
        }

        // What can be done.
        ui.add_space(4.0);
        let any = overview.as_ref().is_some_and(|o| !o.backups.is_empty());
        let older = overview.as_ref().map_or(0, Overview::older);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!dirty, egui::Button::new("Back up now"))
                .on_hover_text("A copy of the vault as saved, kept until you delete it")
                .on_disabled_hover_text(SAVE_FIRST)
                .clicked()
            {
                self.back_up_now();
            }
            if dirty {
                ui.label(RichText::new(SAVE_FIRST).small().color(warn));
            }
            if ui
                .add_enabled(!dirty && any, egui::Button::new("Restore…"))
                .on_disabled_hover_text(if dirty { SAVE_FIRST } else { "No backups yet" })
                .clicked()
            {
                self.backups.picking = true;
            }
            let open = overview.as_ref().and_then(|o| o.open.clone());
            if ui
                .add_enabled(open.is_some(), egui::Button::new("Open folder"))
                .clicked()
                && let Some(folder) = open
                && let Err(e) = open_in_explorer(&folder)
            {
                self.backups_result(e, true);
            }
            if !self.backups.old.is_empty()
                && ui
                    .button("Move old .bak")
                    .on_hover_text("Move copies beside the vault into the backup folder")
                    .clicked()
            {
                self.backups.migration = Some(self.backups.old.clone());
            }
        });
        ui.label(RichText::new(OPEN_FOLDER_HINT).small().weak());
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !busy && older > 0,
                    egui::Button::new(format!("Delete backups with older credentials ({older})")),
                )
                .clicked()
            {
                self.backups.ask = Some(Ask::DeleteOlder(older));
            }
            if ui
                .add_enabled(!busy && any, egui::Button::new("Delete all backups"))
                .clicked()
            {
                let n = overview.as_ref().map_or(0, |o| o.backups.len());
                self.backups.ask = Some(Ask::DeleteAll(n));
            }
        });

        ui.add_space(4.0);
        for line in ABOUT.iter().chain([&RETENTION]) {
            ui.label(RichText::new(format!("•  {line}")).small().weak());
        }
    }

    /// The section's dialogs, for both layouts: the Classic Backups dialog, the restore picker,
    /// the confirmations and the prompt to move old files. Shown in that order, so the later
    /// ones sit on top.
    pub(super) fn backups_dialogs(&mut self, ctx: &egui::Context) {
        if self.backups.dialog {
            let modal = egui::Modal::new(egui::Id::new("backups-dialog")).show(ctx, |ui| {
                ui.set_width(560.0);
                ui.heading("Backups");
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() * 0.7)
                    .show(ui, |ui| self.backups_section(ui));
                ui.add_space(8.0);
                ui.button("Close").clicked()
            });
            if modal.inner || modal.should_close() {
                self.backups.dialog = false;
            }
        }
        if self.backups.picking {
            self.restore_picker(ctx);
        }
        if self.backups.ask.is_some() {
            self.backups_ask(ctx);
        }
        if self.backups.migration.is_some() {
            self.migration_prompt(ctx);
        }
    }

    /// "Restore…": the backups in the known folders, newest first. Nothing else can be picked.
    fn restore_picker(&mut self, ctx: &egui::Context) {
        let backups = self
            .backups
            .overview
            .as_ref()
            .map(|o| o.backups.clone())
            .unwrap_or_default();
        let dirty = self.vault.is_dirty();
        let mut chosen = None;
        let modal = egui::Modal::new(egui::Id::new("backups-restore")).show(ctx, |ui| {
            ui.set_width(600.0);
            ui.heading("Restore a backup");
            ui.add_space(6.0);
            ui.label(RichText::new(RESTORE_WARNING).color(ui.visuals().warn_fg_color));
            ui.label(
                RichText::new("The restored version is saved with your current credentials, and the version it replaces is kept as a backup.")
                    .small()
                    .weak(),
            );
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .max_height(ctx.content_rect().height() * 0.55)
                .show(ui, |ui| {
                    for listed in &backups {
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(!dirty, egui::Button::new("Restore").small())
                                .on_hover_text(listed.backup.file_name.as_str())
                                .clicked()
                            {
                                chosen = Some((listed.backup.path.clone(), listed.when()));
                            }
                            ui.label(RichText::new(listed.when()).small().strong());
                            ui.label(RichText::new(kind_text(listed.kind())).small());
                            ui.label(RichText::new(listed.key.text()).small().weak());
                        });
                    }
                    if backups.is_empty() {
                        ui.label(RichText::new("No backups yet.").small().weak());
                    }
                });
            ui.add_space(8.0);
            ui.button("Cancel").clicked()
        });
        if let Some((path, when)) = chosen {
            self.restore_backup(&path, &when);
        } else if modal.inner || modal.should_close() {
            self.backups.picking = false;
        }
    }

    fn backups_ask(&mut self, ctx: &egui::Context) {
        let Some(ask) = &self.backups.ask else {
            return;
        };
        let (title, body, ok) = match ask {
            Ask::DeleteOlder(n) => (
                "Delete backups with older credentials?".to_owned(),
                format!(
                    "Deletes {} that need an earlier password or backup key file, in every backup folder Axentra can reach. The vault and its current backups are not changed. This can't be undone.",
                    plural(*n, "backup", "backups")
                ),
                format!("Delete {n}"),
            ),
            Ask::DeleteAll(n) => (
                "Delete all backups?".to_owned(),
                format!(
                    "Deletes all {} of this vault in every backup folder Axentra can reach. The vault itself is not changed. This can't be undone.",
                    plural(*n, "backup", "backups")
                ),
                "Delete all".to_owned(),
            ),
            Ask::Root(path, warnings) => (
                "Use this folder for backups?".to_owned(),
                std::iter::once(path.display().to_string())
                    .chain(warnings.iter().map(RootWarning::text))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                "Use this folder".to_owned(),
            ),
            Ask::Forget(path) => (
                "Forget this backup folder?".to_owned(),
                format!(
                    "{}\n\nAxentra stops listing, cleaning up and re-encrypting the backups there, and stops checking the folder at every save and unlock. The backups stay where they are, with the credentials they have now; delete them yourself if you no longer want them.",
                    path.display()
                ),
                "Forget".to_owned(),
            ),
        };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("backups-ask")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading(title);
            ui.add_space(6.0);
            ui.label(body);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new(ok).strong()).clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
        let Some(go) = choice else {
            return;
        };
        let ask = self.backups.ask.take();
        if !go {
            return;
        }
        match ask {
            Some(Ask::DeleteOlder(_)) => self.start_delete(ctx, false),
            Some(Ask::DeleteAll(_)) => self.start_delete(ctx, true),
            Some(Ask::Root(path, _)) => self.set_backup_root(Some(path)),
            Some(Ask::Forget(path)) => self.forget_backup_root(&path),
            None => {}
        }
    }

    /// Asks whether to move old copies from beside the database into the backup folder.
    fn migration_prompt(&mut self, ctx: &egui::Context) {
        let Some(files) = &self.backups.migration else {
            return;
        };
        let location = self
            .vault
            .backup_config()
            .map(|c| c.home().path.display().to_string())
            .unwrap_or_default();
        let names: Vec<String> = files.iter().map(|f| file_name(&f.path)).collect();
        let passkey_files = files.iter().any(|f| !f.database);
        let mut choice = None;
        egui::Modal::new(egui::Id::new("backups-migrate")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.heading("Move old copies into the backup folder?");
            ui.add_space(6.0);
            ui.label("These files beside your vault sync along with it wherever it goes:");
            for name in &names {
                ui.label(RichText::new(format!("•  {name}")).small().strong());
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Each copy of the vault is moved into the backup folder ({location}) and checked there before the original is deleted."
                ))
                .small(),
            );
            if passkey_files {
                ui.label(
                    RichText::new("Old passkey files (.axvault.old) are deleted: they may hold keys you revoked.")
                        .small(),
                );
            }
            ui.label(
                RichText::new("OneDrive keeps moved files in its recycle bin and version history for a while.")
                    .small()
                    .weak(),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Move").strong()).clicked() {
                    choice = Some(0);
                }
                if ui.button("Not now").clicked() {
                    choice = Some(1);
                }
                if ui.button("Don't ask again").clicked() {
                    choice = Some(2);
                }
            });
        });
        let Some(choice) = choice else {
            return;
        };
        self.backups.migration = None;
        match choice {
            0 => self.migrate_old_files(),
            2 => self.vault.decline_migration(),
            _ => {}
        }
    }
}
