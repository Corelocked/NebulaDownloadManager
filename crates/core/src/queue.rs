use std::fs;
use std::path::Path;

use shared::{AppSnapshot, DownloadKind, DownloadRecord, DownloadRequest, DownloadStatus};

use crate::categories::default_download_categories;

pub fn sample_snapshot(downloads_root: &str) -> AppSnapshot {
    let categories = default_download_categories();
    let queue = vec![
        DownloadRecord {
            id: 1,
            request: DownloadRequest::new(
                "ubuntu-24.04.iso".to_owned(),
                "https://releases.ubuntu.com/24.04/ubuntu.iso".to_owned(),
                DownloadKind::Direct,
            ),
            status: DownloadStatus::Downloading,
            progress_percent: 61.4,
            downloaded_mb: 2890.0,
            total_mb: 4707.0,
            speed_mbps: 22.4,
            eta_text: "1m 48s".to_owned(),
            added_from_browser: true,
        },
        DownloadRecord {
            id: 2,
            request: DownloadRequest::new(
                "blender-setup.exe".to_owned(),
                "https://download.blender.org/release/blender.exe".to_owned(),
                DownloadKind::Direct,
            ),
            status: DownloadStatus::Queued,
            progress_percent: 0.0,
            downloaded_mb: 0.0,
            total_mb: 312.0,
            speed_mbps: 0.0,
            eta_text: "Waiting".to_owned(),
            added_from_browser: true,
        },
        DownloadRecord {
            id: 3,
            request: DownloadRequest::new(
                "debian-netinst.torrent".to_owned(),
                "magnet:?xt=urn:btih:abcdef123456".to_owned(),
                DownloadKind::Torrent,
            ),
            status: DownloadStatus::Completed,
            progress_percent: 100.0,
            downloaded_mb: 643.0,
            total_mb: 643.0,
            speed_mbps: 0.0,
            eta_text: "Done".to_owned(),
            added_from_browser: false,
        },
        DownloadRecord {
            id: 4,
            request: DownloadRequest::new(
                "project-spec.pdf".to_owned(),
                "https://example.com/spec.pdf".to_owned(),
                DownloadKind::Direct,
            ),
            status: DownloadStatus::Completed,
            progress_percent: 100.0,
            downloaded_mb: 14.6,
            total_mb: 14.6,
            speed_mbps: 0.0,
            eta_text: "Done".to_owned(),
            added_from_browser: false,
        },
    ];

    AppSnapshot {
        downloads_root: downloads_root.to_owned(),
        categories,
        queue,
    }
}

pub fn total_downloaded_mb(snapshot: &AppSnapshot) -> f32 {
    snapshot.queue.iter().map(|item| item.downloaded_mb).sum()
}

pub fn active_count(snapshot: &AppSnapshot) -> usize {
    snapshot
        .queue
        .iter()
        .filter(|item| {
            matches!(
                item.status,
                DownloadStatus::Downloading | DownloadStatus::Paused
            )
        })
        .count()
}

pub struct QueueManager {
    snapshot: AppSnapshot,
    next_id: u64,
}

impl QueueManager {
    pub fn new(snapshot: AppSnapshot) -> Self {
        let next_id = snapshot.queue.iter().map(|item| item.id).max().unwrap_or(0) + 1;
        Self { snapshot, next_id }
    }

    pub fn snapshot(&self) -> &AppSnapshot {
        &self.snapshot
    }

    pub fn get_record(&self, id: u64) -> Option<&DownloadRecord> {
        self.snapshot.queue.iter().find(|item| item.id == id)
    }

    pub fn add_download(
        &mut self,
        file_name: String,
        source: String,
        kind: DownloadKind,
        added_from_browser: bool,
    ) -> u64 {
        self.add_download_request(
            DownloadRequest::new(file_name, source, kind),
            added_from_browser,
        )
    }

    pub fn add_download_request(
        &mut self,
        request: DownloadRequest,
        added_from_browser: bool,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        self.snapshot.queue.insert(
            0,
            DownloadRecord {
                id,
                request,
                status: DownloadStatus::Queued,
                progress_percent: 0.0,
                downloaded_mb: 0.0,
                total_mb: 0.0,
                speed_mbps: 0.0,
                eta_text: "Waiting".to_owned(),
                added_from_browser,
            },
        );

        id
    }

    pub fn pause(&mut self, id: u64) {
        if let Some(item) = self.snapshot.queue.iter_mut().find(|item| item.id == id) {
            if item.status == DownloadStatus::Downloading {
                item.status = DownloadStatus::Paused;
                item.speed_mbps = 0.0;
                item.eta_text = "Paused".to_owned();
            }
        }
    }

    pub fn resume(&mut self, id: u64) {
        if let Some(item) = self.snapshot.queue.iter_mut().find(|item| item.id == id) {
            if matches!(item.status, DownloadStatus::Paused | DownloadStatus::Queued) {
                item.status = DownloadStatus::Downloading;
                item.speed_mbps = if item.speed_mbps <= 0.0 {
                    8.5
                } else {
                    item.speed_mbps
                };
                item.eta_text = "Resuming".to_owned();
            }
        }
    }

    pub fn mark_completed(&mut self, id: u64) {
        if let Some(item) = self.snapshot.queue.iter_mut().find(|item| item.id == id) {
            item.status = DownloadStatus::Completed;
            item.progress_percent = 100.0;
            item.downloaded_mb = item.total_mb.max(item.downloaded_mb);
            item.speed_mbps = 0.0;
            item.eta_text = "Done".to_owned();
        }
    }

    pub fn clear_browser_metadata(&mut self, id: u64) {
        if let Some(item) = self.snapshot.queue.iter_mut().find(|item| item.id == id) {
            item.request.clear_browser_context();
        }
    }

    pub fn redact_torrent_source(&mut self, id: u64) {
        if let Some(item) = self.snapshot.queue.iter_mut().find(|item| item.id == id) {
            item.request.redact_source_for_history();
        }
    }

    pub fn clear_all_history(&mut self) {
        self.snapshot.queue.clear();
    }

    pub fn fail(&mut self, id: u64, reason: &str) {
        if let Some(item) = self.snapshot.queue.iter_mut().find(|item| item.id == id) {
            item.status = DownloadStatus::Failed;
            item.speed_mbps = 0.0;
            item.eta_text = reason.to_owned();
        }
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.snapshot.queue.len();
        self.snapshot.queue.retain(|item| item.id != id);
        self.snapshot.queue.len() != before
    }

    pub fn apply_download_progress(
        &mut self,
        id: u64,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        bytes_per_second: f64,
    ) {
        if let Some(item) = self.snapshot.queue.iter_mut().find(|item| item.id == id) {
            item.status = DownloadStatus::Downloading;
            item.downloaded_mb = downloaded_bytes as f32 / (1024.0 * 1024.0);
            if let Some(total_bytes) = total_bytes {
                item.total_mb = total_bytes as f32 / (1024.0 * 1024.0);
                if total_bytes > 0 {
                    item.progress_percent =
                        ((downloaded_bytes as f64 / total_bytes as f64) * 100.0) as f32;
                }
            }
            item.speed_mbps = (bytes_per_second / (1024.0 * 1024.0)) as f32;
            item.eta_text = if let Some(total_bytes) = total_bytes {
                let remaining = total_bytes.saturating_sub(downloaded_bytes);
                if bytes_per_second > 0.0 {
                    format!("{:.0}s remaining", remaining as f64 / bytes_per_second)
                } else {
                    "Calculating".to_owned()
                }
            } else {
                "Streaming".to_owned()
            };
        }
    }

    pub fn set_total_bytes(&mut self, id: u64, total_bytes: Option<u64>) {
        if let Some(item) = self.snapshot.queue.iter_mut().find(|item| item.id == id) {
            if let Some(total_bytes) = total_bytes {
                item.total_mb = total_bytes as f32 / (1024.0 * 1024.0);
            }
            item.status = DownloadStatus::Downloading;
            item.eta_text = "Starting".to_owned();
        }
    }

    pub fn start_next_queued_direct(&mut self) -> Option<u64> {
        let item = self.snapshot.queue.iter_mut().find(|item| {
            item.request.kind == DownloadKind::Direct && item.status == DownloadStatus::Queued
        })?;

        item.status = DownloadStatus::Downloading;
        item.speed_mbps = 9.2;
        item.total_mb = if item.total_mb <= 0.0 {
            128.0
        } else {
            item.total_mb
        };
        item.eta_text = "Connecting".to_owned();
        Some(item.id)
    }

    pub fn start_next_queued_torrent(&mut self) -> Option<u64> {
        let item = self.snapshot.queue.iter_mut().find(|item| {
            item.request.kind == DownloadKind::Torrent && item.status == DownloadStatus::Queued
        })?;

        item.status = DownloadStatus::Downloading;
        item.speed_mbps = 0.0;
        item.total_mb = if item.total_mb <= 0.0 {
            768.0
        } else {
            item.total_mb
        };
        item.eta_text = "Contacting trackers".to_owned();
        Some(item.id)
    }

    pub fn apply_torrent_progress(
        &mut self,
        id: u64,
        progress_percent: f32,
        downloaded_bytes: u64,
        total_bytes: u64,
        download_rate_mbps: f32,
        peers: u32,
        eta_text: &str,
    ) {
        if let Some(item) = self.snapshot.queue.iter_mut().find(|item| item.id == id) {
            item.status = DownloadStatus::Downloading;
            item.progress_percent = progress_percent.min(100.0);
            item.downloaded_mb = downloaded_bytes as f32 / (1024.0 * 1024.0);
            item.total_mb = total_bytes as f32 / (1024.0 * 1024.0);
            item.speed_mbps = download_rate_mbps;
            item.eta_text = format!("{eta_text} | {peers} peers");
        }
    }

    pub fn tick_demo_progress(&mut self) {
        let mut completed_ids = Vec::new();

        for item in &mut self.snapshot.queue {
            match item.status {
                DownloadStatus::Downloading => {
                    item.progress_percent = (item.progress_percent + 2.4).min(100.0);
                    if item.total_mb > 0.0 {
                        item.downloaded_mb =
                            (item.total_mb * (item.progress_percent / 100.0)).min(item.total_mb);
                    }
                    if item.progress_percent >= 100.0 {
                        completed_ids.push(item.id);
                    } else {
                        item.eta_text = "Updating".to_owned();
                    }
                }
                DownloadStatus::Queued => {
                    item.eta_text = "Waiting".to_owned();
                }
                _ => {}
            }
        }

        for id in completed_ids {
            self.mark_completed(id);
        }
    }
}

pub fn load_snapshot_or_sample(path: &Path, downloads_root: &str) -> AppSnapshot {
    if let Ok(contents) = fs::read_to_string(path) {
        if let Ok(mut snapshot) = serde_json::from_str::<AppSnapshot>(&contents) {
            for item in &mut snapshot.queue {
                if item.status == DownloadStatus::Seeding {
                    item.status = DownloadStatus::Completed;
                    item.speed_mbps = 0.0;
                    item.eta_text = "Done".to_owned();
                }
            }
            return snapshot;
        }
    }

    sample_snapshot(downloads_root)
}

pub fn save_snapshot(path: &Path, snapshot: &AppSnapshot) -> Result<(), String> {
    let serialized =
        serde_json::to_string_pretty(snapshot).map_err(|err| format!("serialize failed: {err}"))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create dir failed: {err}"))?;
    }

    fs::write(path, serialized).map_err(|err| format!("write failed: {err}"))
}

#[cfg(test)]
mod tests {
    use super::{QueueManager, sample_snapshot};
    use shared::{DownloadKind, DownloadStatus};

    #[test]
    fn add_download_inserts_new_queued_item() {
        let mut manager = QueueManager::new(sample_snapshot("Downloads"));
        let id = manager.add_download(
            "archlinux.iso".to_owned(),
            "https://example.com/archlinux.iso".to_owned(),
            DownloadKind::Direct,
            true,
        );

        let item = manager
            .snapshot()
            .queue
            .iter()
            .find(|item| item.id == id)
            .expect("new item should exist");

        assert_eq!(item.status, DownloadStatus::Queued);
        assert!(item.added_from_browser);
    }

    #[test]
    fn pause_and_resume_update_status() {
        let mut manager = QueueManager::new(sample_snapshot("Downloads"));

        manager.pause(1);
        let paused = manager
            .snapshot()
            .queue
            .iter()
            .find(|item| item.id == 1)
            .expect("sample item should exist");
        assert_eq!(paused.status, DownloadStatus::Paused);

        manager.resume(1);
        let resumed = manager
            .snapshot()
            .queue
            .iter()
            .find(|item| item.id == 1)
            .expect("sample item should exist");
        assert_eq!(resumed.status, DownloadStatus::Downloading);
    }

    #[test]
    fn start_next_queued_direct_promotes_waiting_job() {
        let mut manager = QueueManager::new(sample_snapshot("Downloads"));

        let id = manager
            .start_next_queued_direct()
            .expect("queued job should start");
        let started = manager
            .snapshot()
            .queue
            .iter()
            .find(|item| item.id == id)
            .expect("started item should exist");

        assert_eq!(started.status, DownloadStatus::Downloading);
        assert_eq!(started.eta_text, "Connecting");
    }

    #[test]
    fn apply_download_progress_updates_metrics() {
        let mut manager = QueueManager::new(sample_snapshot("Downloads"));

        manager.apply_download_progress(1, 5 * 1024 * 1024, Some(10 * 1024 * 1024), 2_000_000.0);
        let item = manager.get_record(1).expect("sample item should exist");

        assert_eq!(item.progress_percent, 50.0);
        assert!(item.speed_mbps > 1.0);
    }

    #[test]
    fn start_next_queued_torrent_promotes_waiting_torrent() {
        let mut manager = QueueManager::new(sample_snapshot("Downloads"));
        let id = manager.add_download(
            "archlinux.torrent".to_owned(),
            "magnet:?xt=urn:btih:arch".to_owned(),
            DownloadKind::Torrent,
            false,
        );

        let started = manager
            .start_next_queued_torrent()
            .expect("torrent job should start");
        assert_eq!(started, id);
        assert_eq!(
            manager
                .get_record(id)
                .expect("torrent item should exist")
                .status,
            DownloadStatus::Downloading
        );
    }

    #[test]
    fn remove_deletes_item_from_queue() {
        let mut manager = QueueManager::new(sample_snapshot("Downloads"));

        assert!(manager.remove(2));
        assert!(manager.get_record(2).is_none());
        assert!(!manager.remove(999));
    }
}
