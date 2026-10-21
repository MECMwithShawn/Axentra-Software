//! Serde models for the subset of Microsoft Graph resources OneDrive sync uses.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveItem {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub e_tag: Option<String>,
    #[serde(default)]
    pub c_tag: Option<String>,
    #[serde(default)]
    pub created_date_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_modified_date_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub parent_reference: Option<ItemReference>,
    #[serde(default)]
    pub file: Option<FileFacet>,
    #[serde(default)]
    pub folder: Option<FolderFacet>,
    /// OneNote notebooks and similar bundles; not syncable as plain files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<serde_json::Value>,
    /// "Add shortcut to My files" items that live in someone else's drive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_item: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<serde_json::Value>,
    #[serde(default)]
    pub web_url: Option<String>,
    #[serde(default)]
    pub file_system_info: Option<FileSystemInfo>,
    #[serde(default)]
    pub last_modified_by: Option<IdentitySet>,
    #[serde(
        default,
        rename = "@microsoft.graph.downloadUrl",
        skip_serializing_if = "Option::is_none"
    )]
    pub download_url: Option<String>,
}

impl DriveItem {
    pub fn is_folder(&self) -> bool {
        self.folder.is_some() || self.root.is_some()
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted.is_some()
    }

    pub fn parent_id(&self) -> Option<&str> {
        self.parent_reference.as_ref()?.id.as_deref()
    }

    /// The client-supplied modification time if present (what the Windows
    /// client shows), otherwise the server's.
    pub fn modified(&self) -> Option<DateTime<Utc>> {
        self.file_system_info
            .as_ref()
            .and_then(|f| f.last_modified_date_time)
            .or(self.last_modified_date_time)
    }

    pub fn quick_xor_hash(&self) -> Option<&str> {
        self.file
            .as_ref()?
            .hashes
            .as_ref()?
            .quick_xor_hash
            .as_deref()
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemReference {
    #[serde(default)]
    pub drive_id: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFacet {
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub hashes: Option<Hashes>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hashes {
    #[serde(default)]
    pub quick_xor_hash: Option<String>,
    #[serde(default)]
    pub sha1_hash: Option<String>,
    #[serde(default)]
    pub sha256_hash: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderFacet {
    #[serde(default)]
    pub child_count: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSystemInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_date_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified_date_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySet {
    #[serde(default)]
    pub user: Option<Identity>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub user_principal_name: Option<String>,
    #[serde(default)]
    pub mail: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drive {
    pub id: String,
    /// "personal", "business" or "documentLibrary".
    #[serde(default)]
    pub drive_type: Option<String>,
    #[serde(default)]
    pub owner: Option<IdentitySet>,
    #[serde(default)]
    pub quota: Option<Quota>,
    #[serde(default)]
    pub web_url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub used: Option<u64>,
    #[serde(default)]
    pub remaining: Option<u64>,
    #[serde(default)]
    pub deleted: Option<u64>,
    /// "normal", "nearing", "critical" or "exceeded".
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Permission {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub link: Option<SharingLink>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharingLink {
    #[serde(default)]
    pub web_url: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveItemVersion {
    pub id: String,
    #[serde(default)]
    pub last_modified_date_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub last_modified_by: Option<IdentitySet>,
}

/// One set of preview images OneDrive made for an item (photos, videos,
/// documents). Set 0 is the item's own content.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailSet {
    #[serde(default)]
    pub id: Option<String>,
    /// 96 px on the longest side.
    #[serde(default)]
    pub small: Option<Thumbnail>,
    /// 176 px.
    #[serde(default)]
    pub medium: Option<Thumbnail>,
    /// 800 px.
    #[serde(default)]
    pub large: Option<Thumbnail>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thumbnail {
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// Pre-authenticated and short-lived.
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Collection<T> {
    pub value: Vec<T>,
    #[serde(default, rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    #[serde(default, rename = "@odata.deltaLink")]
    pub delta_link: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UploadSession {
    pub upload_url: String,
}
