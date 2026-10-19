//! The Version history window for a single file.

use std::time::Duration;

use anyhow::Result;
use axentra_onedrive_core::ipc::AxentraOneDriveProxy;
use axentra_onedrive_core::status::human_size;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{VersionRow, VersionsWindow};

pub fn rows(json: &str) -> Vec<VersionRow> {
    let list: Vec<serde_json::Value> = serde_json::from_str(json).unwrap_or_default();
    list.iter()
        .enumerate()
        .map(|(i, v)| {
            let when = v["lastModifiedDateTime"]
                .as_str()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| {
                    t.with_timezone(&chrono::Local)
                        .format("%b %-d, %Y, %-I:%M %p")
                        .to_string()
                })
                .unwrap_or_default();
            VersionRow {
                id: v["id"].as_str().unwrap_or_default().into(),
                when: when.into(),
                who: v["lastModifiedBy"]["user"]["displayName"]
                    .as_str()
                    .unwrap_or("Unknown")
                    .into(),
                size: human_size(v["size"].as_u64().unwrap_or(0)).into(),
                current: i == 0,
            }
        })
        .collect()
}

async fn fetch(path: &str) -> Result<String> {
    let conn = zbus::Connection::session().await?;
    let p = AxentraOneDriveProxy::new(&conn).await?;
    Ok(p.versions(path.to_string()).await?)
}

pub fn run(path: String) -> Result<()> {
    let path = std::path::absolute(&path)?.display().to_string();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    let ui = VersionsWindow::new()?;
    ui.set_file_name(path.rsplit('/').next().unwrap_or(&path).into());
    match rt.block_on(fetch(&path)) {
        Ok(json) => ui.set_versions(ModelRc::new(VecModel::from(rows(&json)))),
        Err(e) => ui.set_message(format!("Couldn't load the version history: {e}").into()),
    }

    let weak = ui.as_weak();
    let handle = rt.handle().clone();
    ui.on_restore(move |id| {
        let (weak, path, id) = (weak.clone(), path.clone(), id.to_string());
        handle.spawn(async move {
            let result = async {
                let conn = zbus::Connection::session().await?;
                let p = AxentraOneDriveProxy::new(&conn).await?;
                p.restore_version(path.clone(), id).await?;
                // Give the service a moment to pick up the new current version.
                tokio::time::sleep(Duration::from_millis(500)).await;
                Ok::<_, anyhow::Error>(p.versions(path).await?)
            }
            .await;
            let _ = weak.upgrade_in_event_loop(move |ui| match result {
                Ok(json) => {
                    ui.set_versions(ModelRc::new(VecModel::from(rows(&json))));
                    ui.set_message(
                        "Restored. The version it replaced is still in the list.".into(),
                    );
                }
                Err(e) => ui.set_message(format!("Couldn't restore that version: {e}").into()),
            });
        });
    });
    ui.run()?;
    Ok(())
}
