use shared::{
    DownloadCategory, DownloadPlan, DownloadRequest, TorrentFileEntry, TorrentSessionSnapshot,
};

use crate::planner::plan_download;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentTaskPlan {
    pub target: DownloadPlan,
    pub session_file_path: String,
    pub data_root: String,
    pub magnet_uri: String,
    pub suggested_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TorrentProgress {
    pub progress_percent: f32,
    pub download_rate_mbps: f32,
    pub upload_rate_mbps: f32,
    pub connected_peers: u32,
    pub eta_text: String,
}

pub fn build_torrent_task_plan(
    request: &DownloadRequest,
    downloads_root: &str,
    categories: &[DownloadCategory],
) -> TorrentTaskPlan {
    let target = plan_download(request, downloads_root, categories);
    let suggested_name = sanitize_torrent_name(&request.file_name);
    let data_root = format!("{}/{}", target.target_folder, suggested_name);
    let session_file_path = format!("{data_root}.torrent-session.json");

    TorrentTaskPlan {
        target,
        session_file_path,
        data_root,
        magnet_uri: request.source.clone(),
        suggested_name,
    }
}

pub fn create_torrent_session_snapshot(plan: &TorrentTaskPlan) -> TorrentSessionSnapshot {
    TorrentSessionSnapshot {
        info_hash: extract_info_hash(&plan.magnet_uri),
        display_name: plan.suggested_name.clone(),
        save_path: plan.data_root.clone(),
        magnet_uri: plan.magnet_uri.clone(),
        files: vec![
            TorrentFileEntry {
                path: format!("{}/part-1.bin", plan.suggested_name),
                size_bytes: 512 * 1024 * 1024,
            },
            TorrentFileEntry {
                path: format!("{}/part-2.bin", plan.suggested_name),
                size_bytes: 256 * 1024 * 1024,
            },
        ],
        piece_count: 768,
        completed_pieces: 0,
        downloaded_bytes: 0,
        uploaded_bytes: 0,
        connected_peers: 0,
        trackers: default_trackers(),
    }
}

pub fn simulate_torrent_progress(
    session: &mut TorrentSessionSnapshot,
    newly_completed_pieces: u32,
    peers: u32,
) -> TorrentProgress {
    session.completed_pieces =
        (session.completed_pieces + newly_completed_pieces).min(session.piece_count);
    session.connected_peers = peers;
    session.downloaded_bytes = (session.completed_pieces as u64) * 1024 * 1024;
    session.uploaded_bytes += (peers as u64) * 256 * 1024;

    let progress_percent = if session.piece_count == 0 {
        0.0
    } else {
        (session.completed_pieces as f32 / session.piece_count as f32) * 100.0
    };

    TorrentProgress {
        progress_percent,
        download_rate_mbps: peers as f32 * 0.8,
        upload_rate_mbps: peers as f32 * 0.15,
        connected_peers: peers,
        eta_text: if progress_percent >= 100.0 {
            "Seeding".to_owned()
        } else {
            format!(
                "{} pieces left",
                session.piece_count.saturating_sub(session.completed_pieces)
            )
        },
    }
}

fn extract_info_hash(magnet_uri: &str) -> String {
    magnet_uri
        .split("btih:")
        .nth(1)
        .and_then(|tail| tail.split('&').next())
        .unwrap_or("pending-info-hash")
        .to_owned()
}

fn sanitize_torrent_name(name: &str) -> String {
    let cleaned = name.replace(['<', '>', ':', '"', '/', '\\', '|', '?', '*'], "_");
    cleaned.trim_end_matches(".torrent").to_owned()
}

fn default_trackers() -> Vec<String> {
    vec![
        "udp://tracker.opentrackr.org:1337/announce".to_owned(),
        "udp://tracker.openbittorrent.com:6969/announce".to_owned(),
        "udp://tracker.torrent.eu.org:451/announce".to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use shared::{DownloadKind, DownloadRequest};

    use crate::categories::default_download_categories;

    use super::{
        build_torrent_task_plan, create_torrent_session_snapshot, simulate_torrent_progress,
    };

    #[test]
    fn torrent_plan_routes_to_torrent_folder() {
        let categories = default_download_categories();
        let request = DownloadRequest::new(
            "debian-netinst.torrent".to_owned(),
            "magnet:?xt=urn:btih:abcdef123456".to_owned(),
            DownloadKind::Torrent,
        );

        let plan = build_torrent_task_plan(&request, "Downloads", &categories);

        assert_eq!(plan.target.category_name, "Torrents");
        assert!(plan.session_file_path.contains("torrent-session"));
    }

    #[test]
    fn torrent_progress_moves_session_forward() {
        let categories = default_download_categories();
        let request = DownloadRequest::new(
            "ubuntu.torrent".to_owned(),
            "magnet:?xt=urn:btih:123456".to_owned(),
            DownloadKind::Torrent,
        );

        let plan = build_torrent_task_plan(&request, "Downloads", &categories);
        let mut session = create_torrent_session_snapshot(&plan);
        let progress = simulate_torrent_progress(&mut session, 64, 18);

        assert!(progress.progress_percent > 0.0);
        assert_eq!(session.connected_peers, 18);
        assert_eq!(session.completed_pieces, 64);
    }
}
