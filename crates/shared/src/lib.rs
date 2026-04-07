use std::fmt::{Display, Formatter};
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DownloadKind {
    Direct,
    Torrent,
}

impl Display for DownloadKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct => write!(f, "Direct"),
            Self::Torrent => write!(f, "Torrent"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadRequest {
    pub file_name: String,
    pub source: String,
    pub kind: DownloadKind,
    pub custom_target_folder: Option<String>,
    pub referrer: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    pub user_agent: Option<String>,
    pub cookie_header: Option<String>,
    #[serde(default)]
    pub secondary_source: Option<String>,
    #[serde(default)]
    pub source_mime_type: Option<String>,
    #[serde(default)]
    pub secondary_source_mime_type: Option<String>,
    #[serde(default)]
    pub capture_diagnostics: Vec<String>,
    #[serde(default)]
    pub use_yt_dlp: bool,
}

impl DownloadRequest {
    pub fn new(file_name: String, source: String, kind: DownloadKind) -> Self {
        Self {
            file_name,
            source,
            kind,
            custom_target_folder: None,
            referrer: None,
            origin: None,
            user_agent: None,
            cookie_header: None,
            secondary_source: None,
            source_mime_type: None,
            secondary_source_mime_type: None,
            capture_diagnostics: Vec::new(),
            use_yt_dlp: false,
        }
    }

    pub fn with_browser_context(
        mut self,
        referrer: Option<String>,
        origin: Option<String>,
        user_agent: Option<String>,
        cookie_header: Option<String>,
    ) -> Self {
        self.referrer = referrer;
        self.origin = origin;
        self.user_agent = user_agent;
        self.cookie_header = cookie_header;
        self
    }

    pub fn with_secondary_source(
        mut self,
        secondary_source: Option<String>,
        source_mime_type: Option<String>,
        secondary_source_mime_type: Option<String>,
    ) -> Self {
        self.secondary_source = secondary_source;
        self.source_mime_type = source_mime_type;
        self.secondary_source_mime_type = secondary_source_mime_type;
        self
    }

    pub fn with_capture_diagnostics(mut self, capture_diagnostics: Vec<String>) -> Self {
        self.capture_diagnostics = capture_diagnostics;
        self
    }

    pub fn with_yt_dlp(mut self, use_yt_dlp: bool) -> Self {
        self.use_yt_dlp = use_yt_dlp;
        self
    }

    pub fn clear_browser_context(&mut self) {
        self.referrer = None;
        self.origin = None;
        self.user_agent = None;
        self.cookie_header = None;
    }

    pub fn redact_source_for_history(&mut self) {
        if self.kind == DownloadKind::Torrent {
            self.source = "[magnet redacted]".to_owned();
        }
    }

    pub fn with_custom_target_folder(mut self, target_folder: Option<String>) -> Self {
        self.custom_target_folder = target_folder;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadCategory {
    pub name: String,
    pub folder_name: String,
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadPlan {
    pub category_name: String,
    pub target_folder: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Seeding,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DuplicateStrategy {
    Rename,
    Overwrite,
    Skip,
}

impl Display for DuplicateStrategy {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rename => write!(f, "Rename"),
            Self::Overwrite => write!(f, "Overwrite"),
            Self::Skip => write!(f, "Skip"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PostDownloadAction {
    None,
    OpenFile,
    OpenFolder,
}

impl Display for PostDownloadAction {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::OpenFile => write!(f, "Open File"),
            Self::OpenFolder => write!(f, "Open Folder"),
        }
    }
}

impl Display for DownloadStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => write!(f, "Queued"),
            Self::Downloading => write!(f, "Downloading"),
            Self::Seeding => write!(f, "Seeding"),
            Self::Paused => write!(f, "Paused"),
            Self::Completed => write!(f, "Completed"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum QueueView {
    Active,
    Queued,
    Completed,
    Torrents,
    BrowserCapture,
}

impl QueueView {
    pub const ALL: [QueueView; 5] = [
        QueueView::Active,
        QueueView::Queued,
        QueueView::Completed,
        QueueView::Torrents,
        QueueView::BrowserCapture,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Queued => "Queued",
            Self::Completed => "Completed",
            Self::Torrents => "Torrents",
            Self::BrowserCapture => "Browser Capture",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DownloadRecord {
    pub id: u64,
    pub request: DownloadRequest,
    pub status: DownloadStatus,
    pub progress_percent: f32,
    pub downloaded_mb: f32,
    pub total_mb: f32,
    pub speed_mbps: f32,
    pub eta_text: String,
    pub added_from_browser: bool,
}

impl DownloadRecord {
    pub fn is_visible_in(&self, view: QueueView) -> bool {
        match view {
            QueueView::Active => matches!(
                self.status,
                DownloadStatus::Downloading | DownloadStatus::Paused
            ),
            QueueView::Queued => self.status == DownloadStatus::Queued,
            QueueView::Completed => self.status == DownloadStatus::Completed,
            QueueView::Torrents => self.request.kind == DownloadKind::Torrent,
            QueueView::BrowserCapture => self.added_from_browser,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSnapshot {
    pub downloads_root: String,
    pub categories: Vec<DownloadCategory>,
    pub queue: Vec<DownloadRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TorrentFileEntry {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TorrentSessionSnapshot {
    pub info_hash: String,
    pub display_name: String,
    pub save_path: String,
    pub magnet_uri: String,
    pub files: Vec<TorrentFileEntry>,
    pub piece_count: u32,
    pub completed_pieces: u32,
    pub downloaded_bytes: u64,
    pub uploaded_bytes: u64,
    pub connected_peers: u32,
    pub trackers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserCapturePayload {
    pub file_name: String,
    pub source: String,
    pub kind: DownloadKind,
    #[serde(default)]
    pub auto_start: bool,
    pub referrer: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    pub user_agent: Option<String>,
    pub cookie_header: Option<String>,
    #[serde(default)]
    pub secondary_source: Option<String>,
    #[serde(default)]
    pub source_mime_type: Option<String>,
    #[serde(default)]
    pub secondary_source_mime_type: Option<String>,
    #[serde(default)]
    pub capture_diagnostics: Vec<String>,
    #[serde(default)]
    pub use_yt_dlp: bool,
}

impl BrowserCapturePayload {
    pub fn into_request(self) -> DownloadRequest {
        DownloadRequest::new(self.file_name, self.source, self.kind)
            .with_browser_context(
                self.referrer,
                self.origin,
                self.user_agent,
                self.cookie_header,
            )
            .with_secondary_source(
                self.secondary_source,
                self.source_mime_type,
                self.secondary_source_mime_type,
            )
            .with_capture_diagnostics(self.capture_diagnostics)
            .with_yt_dlp(self.use_yt_dlp)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacySettings {
    pub privacy_mode: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self { privacy_mode: true }
    }
}

impl PrivacySettings {
    pub fn auto_stop_on_completion(&self) -> bool {
        self.privacy_mode
    }

    pub fn no_seeding(&self) -> bool {
        self.privacy_mode
    }

    pub fn disable_peer_discovery_extras(&self) -> bool {
        self.privacy_mode
    }

    pub fn minimize_browser_metadata_retention(&self) -> bool {
        self.privacy_mode
    }

    pub fn minimize_logging(&self) -> bool {
        self.privacy_mode
    }

    pub fn minimize_torrent_metadata_retention(&self) -> bool {
        self.privacy_mode
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesktopPersistedState {
    pub expanded_details_job_id: Option<u64>,
    pub privacy: PrivacySettings,
    pub run_on_startup: bool,
    pub clipboard_watch_enabled: bool,
    pub native_notifications_enabled: bool,
    pub update_feed_url: String,
    pub duplicate_strategy: DuplicateStrategy,
    pub post_download_action: PostDownloadAction,
    pub rqbit: Option<RqbitPersistedState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RqbitPersistedState {
    pub queue_job_id: u64,
    pub magnet_uri: String,
    pub torrent_name: Option<String>,
    pub info_hash: Option<String>,
    pub output_folder: Option<String>,
    pub file_count: Option<usize>,
    pub files: Vec<TorrentFileEntry>,
    pub peer_count: u32,
}

impl RqbitPersistedState {
    pub fn matches_torrent_job(&self, request: &DownloadRequest) -> bool {
        request.kind == DownloadKind::Torrent && request.source == self.magnet_uri
    }
}

impl DesktopPersistedState {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let serialized =
            serde_json::to_string_pretty(self).map_err(|err| format!("serialize failed: {err}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| format!("create dir failed: {err}"))?;
        }
        std::fs::write(path, serialized).map_err(|err| format!("write failed: {err}"))
    }
}

impl Default for DesktopPersistedState {
    fn default() -> Self {
        Self {
            expanded_details_job_id: None,
            privacy: PrivacySettings::default(),
            run_on_startup: true,
            clipboard_watch_enabled: true,
            native_notifications_enabled: true,
            update_feed_url: String::new(),
            duplicate_strategy: DuplicateStrategy::Rename,
            post_download_action: PostDownloadAction::OpenFolder,
            rqbit: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DesktopPersistedState, DownloadKind, DownloadRequest, RqbitPersistedState, TorrentFileEntry,
    };

    #[test]
    fn rqbit_state_matches_same_magnet_torrent() {
        let state = RqbitPersistedState {
            queue_job_id: 7,
            magnet_uri: "magnet:?xt=urn:btih:123".to_owned(),
            torrent_name: Some("ubuntu".to_owned()),
            info_hash: Some("123".to_owned()),
            output_folder: Some("Downloads/Torrents/ubuntu".to_owned()),
            file_count: Some(3),
            files: vec![TorrentFileEntry {
                path: "ubuntu/file.iso".to_owned(),
                size_bytes: 1024,
            }],
            peer_count: 14,
        };
        let request = DownloadRequest::new(
            "ubuntu.torrent".to_owned(),
            "magnet:?xt=urn:btih:123".to_owned(),
            DownloadKind::Torrent,
        );

        assert!(state.matches_torrent_job(&request));
    }

    #[test]
    fn rqbit_state_rejects_changed_source() {
        let state = RqbitPersistedState {
            queue_job_id: 7,
            magnet_uri: "magnet:?xt=urn:btih:123".to_owned(),
            torrent_name: None,
            info_hash: None,
            output_folder: None,
            file_count: None,
            files: Vec::new(),
            peer_count: 0,
        };
        let request = DownloadRequest::new(
            "ubuntu.torrent".to_owned(),
            "magnet:?xt=urn:btih:456".to_owned(),
            DownloadKind::Torrent,
        );

        assert!(!state.matches_torrent_job(&request));
    }

    #[test]
    fn desktop_state_load_defaults_when_missing() {
        let temp_path = std::env::temp_dir().join("nebula_dm_missing_desktop_state.json");
        let _ = std::fs::remove_file(&temp_path);

        let loaded = DesktopPersistedState::load(&temp_path);

        assert_eq!(loaded, DesktopPersistedState::default());
    }
}
