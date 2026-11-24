//! What the manifest says, for people deciding whether the mirror can be
//! relied on (and later for `MIGRATION_READINESS.md`).
//!
//! Every figure is put in one of three groups, and never rounded up from
//! one to the next:
//!
//! - **measured**: bytes were hashed here and matched. L1 for the staged
//!   source copy; L4 for what the destination actually stores.
//! - **claimed**: the destination says so (L3). Proves the metadata it
//!   recorded, not the bytes it stores.
//! - **not verified**: uploaded with no confirmation (L2), or not mirrored.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use axentra_clouddrive_provider::{ItemKind, VerifyLevel};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::MResult;
use crate::manifest::{ItemRow, ItemState, Manifest, RunRow};
use crate::planner::{human_bytes, Count};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub path: String,
    pub state: ItemState,
    pub level: VerifyLevel,
    pub attempts: u32,
    pub detail: Option<String>,
    pub last_error: Option<String>,
    pub next_attempt_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readiness {
    pub generated: DateTime<Utc>,
    pub dest_root: Option<String>,
    pub folders: BTreeMap<ItemState, u64>,
    pub files_by_state: BTreeMap<ItemState, Count>,
    /// Files still on the source, by the highest level reached.
    pub files_by_level: BTreeMap<VerifyLevel, Count>,
    /// Files still on the source that the mirror means to copy (not
    /// skipped, not diverged).
    pub eligible: Count,
    /// Stored bytes measured to match (L4).
    pub measured_round_trip: Count,
    /// Destination's claim matches, stored bytes not checked (exactly L3).
    pub claimed_only: Count,
    /// Uploaded with nothing confirmed (exactly L2), or not uploaded yet.
    pub not_verified: Count,
    /// Uploads whose outcome is unknown; the next run checks first.
    pub uploads_pending: u64,
    pub diverged: Vec<Problem>,
    pub failed: Vec<Problem>,
    pub skipped: Vec<Problem>,
    pub last_run: Option<RunRow>,
}

impl Readiness {
    pub fn from_manifest(m: &Manifest) -> MResult<Self> {
        let mut r = Self::from_rows(&m.items()?, Utc::now());
        r.dest_root = m.kv_get("dest_root")?;
        r.last_run = m.runs(1)?.into_iter().next();
        Ok(r)
    }

    pub fn from_rows(rows: &[ItemRow], generated: DateTime<Utc>) -> Self {
        let mut r = Self {
            generated,
            dest_root: None,
            folders: BTreeMap::new(),
            files_by_state: BTreeMap::new(),
            files_by_level: BTreeMap::new(),
            eligible: Count::default(),
            measured_round_trip: Count::default(),
            claimed_only: Count::default(),
            not_verified: Count::default(),
            uploads_pending: 0,
            diverged: Vec::new(),
            failed: Vec::new(),
            skipped: Vec::new(),
            last_run: None,
        };
        let add = |c: &mut Count, n: u64| {
            c.items += 1;
            c.bytes += n;
        };
        for row in rows {
            if row.kind == ItemKind::Folder {
                *r.folders.entry(row.state).or_default() += 1;
                if row.state == ItemState::Diverged {
                    r.diverged.push(problem(row));
                }
                continue;
            }
            add(r.files_by_state.entry(row.state).or_default(), row.size);
            if row.upload_pending {
                r.uploads_pending += 1;
            }
            match row.state {
                ItemState::SourceDeleted => continue,
                ItemState::Diverged => {
                    r.diverged.push(problem(row));
                    continue;
                }
                ItemState::Skipped => {
                    r.skipped.push(problem(row));
                    continue;
                }
                ItemState::Failed => r.failed.push(problem(row)),
                _ => {}
            }
            add(
                r.files_by_level.entry(row.verify_level).or_default(),
                row.size,
            );
            add(&mut r.eligible, row.size);
            match row.verify_level {
                VerifyLevel::L4RoundTrip => add(&mut r.measured_round_trip, row.size),
                VerifyLevel::L3RemoteClaimed => add(&mut r.claimed_only, row.size),
                _ => add(&mut r.not_verified, row.size),
            }
        }
        r
    }

    pub fn to_json(&self) -> MResult<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn render_text(&self) -> String {
        let mut s = String::new();
        let pct = |c: &Count| {
            if self.eligible.items == 0 {
                "-".to_string()
            } else {
                format!(
                    "{:.1}%",
                    c.items as f64 * 100.0 / self.eligible.items as f64
                )
            }
        };
        let line = |s: &mut String, label: &str, c: &Count| {
            let _ = writeln!(
                s,
                "  {label:<44} {:>7} {:>7} {:>12}",
                c.items,
                pct(c),
                human_bytes(c.bytes)
            );
        };
        let _ = writeln!(
            s,
            "Mirror readiness ({})",
            self.generated.format("%Y-%m-%d %H:%M UTC")
        );
        if let Some(root) = &self.dest_root {
            let _ = writeln!(s, "  Destination folder: /{}", root.trim_start_matches('/'));
        }
        let _ = writeln!(
            s,
            "Files to mirror: {} ({})",
            self.eligible.items,
            human_bytes(self.eligible.bytes)
        );
        line(
            &mut s,
            "Measured: stored bytes match (round trip, L4)",
            &self.measured_round_trip,
        );
        line(
            &mut s,
            "Claimed only: destination SHA-1 matches (L3)",
            &self.claimed_only,
        );
        line(
            &mut s,
            "Not verified (uploaded L2, or not yet)",
            &self.not_verified,
        );
        let _ = writeln!(s, "By level:");
        for (level, c) in &self.files_by_level {
            line(&mut s, level.as_str(), c);
        }
        let _ = writeln!(s, "By state (all files):");
        for (state, c) in &self.files_by_state {
            let _ = writeln!(
                s,
                "  {:<44} {:>7} {:>20}",
                state.as_str(),
                c.items,
                human_bytes(c.bytes)
            );
        }
        if !self.folders.is_empty() {
            let f: Vec<String> = self
                .folders
                .iter()
                .map(|(st, n)| format!("{st} {n}"))
                .collect();
            let _ = writeln!(s, "Folders: {}", f.join(", "));
        }
        if self.uploads_pending > 0 {
            let _ = writeln!(
                s,
                "Uploads with an unknown outcome: {} (checked before any retry)",
                self.uploads_pending
            );
        }
        for (title, list) in [
            ("Failed", &self.failed),
            ("Diverged (both sides left as they are)", &self.diverged),
            ("Skipped", &self.skipped),
        ] {
            if list.is_empty() {
                continue;
            }
            let _ = writeln!(s, "{title}: {}", list.len());
            for p in list.iter().take(20) {
                let why = p
                    .last_error
                    .as_deref()
                    .filter(|_| p.state == ItemState::Failed)
                    .or(p.detail.as_deref())
                    .unwrap_or("");
                let _ = writeln!(s, "  {} [{}] {why}", p.path, p.level.as_str());
            }
            if list.len() > 20 {
                let _ = writeln!(s, "  ... and {} more", list.len() - 20);
            }
        }
        if let Some(run) = &self.last_run {
            let _ = writeln!(
                s,
                "Last run: {} {} -> {}",
                run.mode,
                run.started,
                run.outcome.as_deref().unwrap_or("did not finish")
            );
        }
        s
    }
}

fn problem(row: &ItemRow) -> Problem {
    Problem {
        path: row.source_path.clone(),
        state: row.state,
        level: row.verify_level,
        attempts: row.attempts,
        detail: row.detail.clone(),
        last_error: row.last_error.clone(),
        next_attempt_at: row.next_attempt_at,
    }
}
