//! Stored-thumbnail metadata and change-detection contract.
//!
//! Hovering a library item must not open and render a DWG. The browser UI
//! lives in a later phase; this module defines the cache record, settings,
//! and the metadata/fingerprint checks that later generation will use.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

// ------------------------------------------------------------
// Enum: ThumbnailRefreshPolicy
// Purpose: User choice for stale library previews.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThumbnailRefreshPolicy {
    Ask,
    Automatic,
    Manual,
}

impl Default for ThumbnailRefreshPolicy {
    fn default() -> Self {
        Self::Ask
    }
}

// ------------------------------------------------------------
// Type: ThumbnailSettings
// Purpose: Display, generation, and refresh are independent.
//          Disabling display does not delete images. Disabling
//          generation guarantees hover never decodes a DWG.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailSettings {
    pub show_thumbnails: bool,
    pub generate_missing: bool,
    pub refresh_policy: ThumbnailRefreshPolicy,
}

impl Default for ThumbnailSettings {
    fn default() -> Self {
        Self {
            show_thumbnails: true,
            generate_missing: true,
            refresh_policy: ThumbnailRefreshPolicy::Ask,
        }
    }
}

impl ThumbnailSettings {
    pub fn hover_may_decode_source(&self) -> bool {
        false
    }

    pub fn generation_allowed(&self) -> bool {
        self.generate_missing
    }
}

// ------------------------------------------------------------
// Enum: ThumbnailStatus
// Purpose: Cache state shown to the later library browser.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThumbnailStatus {
    Missing,
    Current,
    Stale,
    Failed,
    Unchecked,
}

// ------------------------------------------------------------
// Type: SourceIdentity
// Purpose: Stable cache key. Filename alone is not unique.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceIdentity {
    pub library_root_id: String,
    pub relative_path: PathBuf,
    pub block_id: Option<u64>,
    pub block_name: Option<String>,
}

impl SourceIdentity {
    pub fn cache_key(&self) -> String {
        let mut key = format!(
            "{}:{}",
            self.library_root_id,
            self.relative_path.to_string_lossy()
        );
        if let Some(id) = self.block_id {
            key.push_str(&format!("#id{id}"));
        } else if let Some(name) = &self.block_name {
            key.push_str(&format!("#{name}"));
        }
        key
    }
}

// ------------------------------------------------------------
// Type: SourceMetadata
// Purpose: Quick filter. Size and timestamp are not proof of
//          identical content.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceMetadata {
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl SourceMetadata {
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        Ok(Self {
            size: meta.len(),
            modified: meta.modified().ok(),
        })
    }

    pub fn matches(self, other: Self) -> bool {
        self.size == other.size && self.modified == other.modified
    }
}

// ------------------------------------------------------------
// Type: ContentFingerprint
// Purpose: Source content identity, stored separately from the
//          fingerprint the current thumbnail was rendered from.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentFingerprint(pub String);

impl ContentFingerprint {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(fnv64a(bytes))
    }
}

fn fnv64a(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

// ------------------------------------------------------------
// Type: ThumbnailRecord
// Purpose: One cached preview plus the metadata needed to detect
//          stale images without decoding the source on hover.
// ------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailRecord {
    pub identity: SourceIdentity,
    pub source_metadata: Option<SourceMetadata>,
    pub current_source_fingerprint: Option<ContentFingerprint>,
    pub thumbnail_source_fingerprint: Option<ContentFingerprint>,
    pub renderer_version: u32,
    pub width: u32,
    pub height: u32,
    pub image_path: Option<PathBuf>,
    pub status: ThumbnailStatus,
    pub last_prompted_fingerprint: Option<ContentFingerprint>,
}

impl ThumbnailRecord {
    pub fn placeholder(identity: SourceIdentity) -> Self {
        Self {
            identity,
            source_metadata: None,
            current_source_fingerprint: None,
            thumbnail_source_fingerprint: None,
            renderer_version: 0,
            width: 0,
            height: 0,
            image_path: None,
            status: ThumbnailStatus::Missing,
            last_prompted_fingerprint: None,
        }
    }

    pub fn is_current(&self) -> bool {
        self.status == ThumbnailStatus::Current
            && self.current_source_fingerprint.is_some()
            && self.current_source_fingerprint == self.thumbnail_source_fingerprint
    }

    /// Metadata change only marks the item as potentially changed.
    pub fn apply_metadata(&mut self, metadata: SourceMetadata) {
        let changed = self
            .source_metadata
            .map(|previous| !previous.matches(metadata))
            .unwrap_or(true);
        self.source_metadata = Some(metadata);
        if changed && self.status == ThumbnailStatus::Current {
            self.status = ThumbnailStatus::Unchecked;
        } else if self.image_path.is_none() {
            self.status = ThumbnailStatus::Missing;
        }
    }

    /// Content fingerprint is compared against the thumbnail's fingerprint.
    /// Detecting a source change must not make the old thumbnail look current.
    pub fn apply_content_fingerprint(&mut self, fingerprint: ContentFingerprint) {
        self.current_source_fingerprint = Some(fingerprint.clone());
        if self.thumbnail_source_fingerprint.as_ref() != Some(&fingerprint) {
            if self.image_path.is_some() {
                self.status = ThumbnailStatus::Stale;
            } else {
                self.status = ThumbnailStatus::Missing;
            }
        } else if self.image_path.is_some() {
            self.status = ThumbnailStatus::Current;
        }
    }

    pub fn should_prompt_refresh(&self, settings: &ThumbnailSettings) -> bool {
        settings.refresh_policy == ThumbnailRefreshPolicy::Ask
            && self.status == ThumbnailStatus::Stale
            && self.last_prompted_fingerprint != self.current_source_fingerprint
    }

    pub fn mark_prompted(&mut self) {
        self.last_prompted_fingerprint = self.current_source_fingerprint.clone();
    }

    /// Generation writes a temp image, then replaces only if the job
    /// fingerprint still matches the current source fingerprint.
    pub fn commit_generated(
        &mut self,
        job_fingerprint: &ContentFingerprint,
        image_path: PathBuf,
        renderer_version: u32,
        width: u32,
        height: u32,
    ) -> bool {
        if self.current_source_fingerprint.as_ref() != Some(job_fingerprint) {
            return false;
        }
        self.thumbnail_source_fingerprint = Some(job_fingerprint.clone());
        self.image_path = Some(image_path);
        self.renderer_version = renderer_version;
        self.width = width;
        self.height = height;
        self.status = ThumbnailStatus::Current;
        true
    }

    pub fn mark_failed_keep_previous(&mut self) {
        if self.image_path.is_some() {
            self.status = ThumbnailStatus::Stale;
        } else {
            self.status = ThumbnailStatus::Failed;
        }
    }
}

pub const THUMBNAIL_FOLDER_NAME: &str = "Thumbnails";

pub fn path_is_thumbnail_folder(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(THUMBNAIL_FOLDER_NAME))
}

pub fn should_scan_asset(path: &Path) -> bool {
    !path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(THUMBNAIL_FOLDER_NAME))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(root: &str, rel: &str) -> SourceIdentity {
        SourceIdentity {
            library_root_id: root.into(),
            relative_path: PathBuf::from(rel),
            block_id: None,
            block_name: Some("Frame".into()),
        }
    }

    #[test]
    fn same_filename_from_different_roots_do_not_share_a_key() {
        let a = identity("lib-a", "blocks/frame.dwg");
        let b = identity("lib-b", "blocks/frame.dwg");
        assert_ne!(a.cache_key(), b.cache_key());
    }

    #[test]
    fn disabling_generation_does_not_allow_source_decode_on_hover() {
        let mut settings = ThumbnailSettings::default();
        settings.generate_missing = false;
        assert!(!settings.generation_allowed());
        assert!(!settings.hover_may_decode_source());
        settings.show_thumbnails = false;
        assert!(!settings.hover_may_decode_source());
    }

    #[test]
    fn metadata_change_does_not_mark_thumbnail_current() {
        let mut record = ThumbnailRecord::placeholder(identity("lib", "a.dwg"));
        record.image_path = Some(PathBuf::from("cache/a.png"));
        record.status = ThumbnailStatus::Current;
        record.thumbnail_source_fingerprint = Some(ContentFingerprint("old".into()));
        record.current_source_fingerprint = Some(ContentFingerprint("old".into()));
        record.apply_metadata(SourceMetadata {
            size: 10,
            modified: None,
        });
        assert_eq!(record.status, ThumbnailStatus::Unchecked);
        record.apply_content_fingerprint(ContentFingerprint("new".into()));
        assert_eq!(record.status, ThumbnailStatus::Stale);
        assert!(!record.is_current());
        assert_eq!(
            record.thumbnail_source_fingerprint,
            Some(ContentFingerprint("old".into()))
        );
    }

    #[test]
    fn ask_policy_does_not_reprompt_the_same_source_version() {
        let settings = ThumbnailSettings {
            show_thumbnails: true,
            generate_missing: true,
            refresh_policy: ThumbnailRefreshPolicy::Ask,
        };
        let mut record = ThumbnailRecord::placeholder(identity("lib", "a.dwg"));
        record.image_path = Some(PathBuf::from("cache/a.png"));
        record.status = ThumbnailStatus::Stale;
        record.current_source_fingerprint = Some(ContentFingerprint("abc".into()));
        assert!(record.should_prompt_refresh(&settings));
        record.mark_prompted();
        assert!(!record.should_prompt_refresh(&settings));
    }

    #[test]
    fn commit_rejects_stale_job_and_keeps_previous_image() {
        let mut record = ThumbnailRecord::placeholder(identity("lib", "a.dwg"));
        record.current_source_fingerprint = Some(ContentFingerprint("new".into()));
        record.image_path = Some(PathBuf::from("cache/old.png"));
        record.status = ThumbnailStatus::Stale;
        let committed = record.commit_generated(
            &ContentFingerprint("old".into()),
            PathBuf::from("cache/new.png"),
            1,
            64,
            64,
        );
        assert!(!committed);
        assert_eq!(record.image_path, Some(PathBuf::from("cache/old.png")));
        record.mark_failed_keep_previous();
        assert_eq!(record.status, ThumbnailStatus::Stale);
    }

    #[test]
    fn thumbnail_folders_are_excluded_from_asset_scanning() {
        assert!(path_is_thumbnail_folder(Path::new("/lib/Thumbnails")));
        assert!(!should_scan_asset(Path::new("/lib/Thumbnails/a.png")));
        assert!(should_scan_asset(Path::new("/lib/blocks/a.dwg")));
    }

    #[test]
    fn multi_block_source_change_invalidates_all_derived_records() {
        let fp = ContentFingerprint::from_bytes(b"dwg-bytes-v2");
        let mut a = ThumbnailRecord::placeholder(SourceIdentity {
            library_root_id: "lib".into(),
            relative_path: PathBuf::from("assembly.dwg"),
            block_id: Some(1),
            block_name: Some("A".into()),
        });
        let mut b = ThumbnailRecord::placeholder(SourceIdentity {
            library_root_id: "lib".into(),
            relative_path: PathBuf::from("assembly.dwg"),
            block_id: Some(2),
            block_name: Some("B".into()),
        });
        a.image_path = Some(PathBuf::from("a.png"));
        b.image_path = Some(PathBuf::from("b.png"));
        a.thumbnail_source_fingerprint = Some(ContentFingerprint::from_bytes(b"dwg-bytes-v1"));
        b.thumbnail_source_fingerprint = Some(ContentFingerprint::from_bytes(b"dwg-bytes-v1"));
        a.apply_content_fingerprint(fp.clone());
        b.apply_content_fingerprint(fp);
        assert_eq!(a.status, ThumbnailStatus::Stale);
        assert_eq!(b.status, ThumbnailStatus::Stale);
    }
}
