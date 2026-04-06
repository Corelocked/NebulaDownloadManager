use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use librqbit::{AddTorrent, AddTorrentResponse, Session, SessionOptions, api::Api};
use shared::{PrivacySettings, TorrentFileEntry};

#[derive(Debug, Clone, PartialEq)]
pub enum RqbitTorrentEvent {
    SessionStarted,
    MetadataResolved {
        display_name: String,
        info_hash: String,
        output_folder: String,
        file_count: usize,
        files: Vec<TorrentFileEntry>,
    },
    Progress {
        progress_percent: f32,
        downloaded_bytes: u64,
        total_bytes: u64,
        download_rate_mbps: f32,
        upload_rate_mbps: f32,
        peers: u32,
        eta_text: String,
    },
    Paused,
    Resumed,
    Removed,
    Deleted,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RqbitTorrentCommand {
    Pause,
    Resume,
    Remove,
    DeleteFiles,
}

pub struct ActiveRqbitTorrent {
    pub events: Receiver<RqbitTorrentEvent>,
    pub commands: Sender<RqbitTorrentCommand>,
}

pub struct RqbitTorrentEngine {
    download_root: PathBuf,
    privacy: PrivacySettings,
}

impl RqbitTorrentEngine {
    pub fn new(download_root: PathBuf, privacy: PrivacySettings) -> Self {
        Self {
            download_root,
            privacy,
        }
    }

    pub fn spawn_magnet_download(&self, magnet_uri: String) -> ActiveRqbitTorrent {
        let (event_sender, event_receiver) = mpsc::channel();
        let (command_sender, command_receiver) = mpsc::channel();
        let download_root = self.download_root.clone();
        let privacy = self.privacy.clone();

        thread::spawn(move || {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(runtime) => runtime,
                Err(err) => {
                    let _ = event_sender.send(RqbitTorrentEvent::Failed(format!(
                        "tokio runtime failed: {err}"
                    )));
                    return;
                }
            };

            runtime.block_on(async move {
                let session = match Session::new_with_opts(download_root, session_options(&privacy)).await {
                    Ok(session) => {
                        let _ = event_sender.send(RqbitTorrentEvent::SessionStarted);
                        session
                    }
                    Err(err) => {
                        let _ = event_sender.send(RqbitTorrentEvent::Failed(format!(
                            "session start failed: {err}"
                        )));
                        return;
                    }
                };

                let response = match session
                    .add_torrent(AddTorrent::from_url(&magnet_uri), None)
                    .await
                {
                    Ok(response) => response,
                    Err(err) => {
                        let _ = event_sender.send(RqbitTorrentEvent::Failed(format!(
                            "add_torrent failed: {err}"
                        )));
                        return;
                    }
                };

                let torrent_id = match response {
                    AddTorrentResponse::Added(id, _handle)
                    | AddTorrentResponse::AlreadyManaged(id, _handle) => id,
                    AddTorrentResponse::ListOnly(_) => {
                        let _ = event_sender.send(RqbitTorrentEvent::Failed(
                            "torrent was added in list-only mode".to_owned(),
                        ));
                        return;
                    }
                };

                let api = Api::new(session.clone(), None);

                if let Ok(details) = api.api_torrent_details(torrent_id.into()) {
                    let files = details
                        .files
                        .unwrap_or_default()
                        .into_iter()
                        .map(|file| TorrentFileEntry {
                            path: if file.components.is_empty() {
                                file.name
                            } else {
                                file.components.join("/")
                            },
                            size_bytes: file.length,
                        })
                        .collect::<Vec<_>>();
                    let _ = event_sender.send(RqbitTorrentEvent::MetadataResolved {
                        display_name: details.name.unwrap_or_else(|| "Unnamed torrent".to_owned()),
                        info_hash: details.info_hash,
                        output_folder: details.output_folder,
                        file_count: files.len(),
                        files,
                    });
                }

                let mut is_paused = false;
                loop {
                    match command_receiver.try_recv() {
                        Ok(RqbitTorrentCommand::Pause) => {
                            match api.api_torrent_action_pause(torrent_id.into()).await {
                                Ok(_) => {
                                    is_paused = true;
                                    let _ = event_sender.send(RqbitTorrentEvent::Paused);
                                }
                                Err(err) => {
                                    let _ = event_sender.send(RqbitTorrentEvent::Failed(format!(
                                        "pause failed: {err}"
                                    )));
                                    return;
                                }
                            }
                        }
                        Ok(RqbitTorrentCommand::Resume) => {
                            match api.api_torrent_action_start(torrent_id.into()).await {
                                Ok(_) => {
                                    is_paused = false;
                                    let _ = event_sender.send(RqbitTorrentEvent::Resumed);
                                }
                                Err(err) => {
                                    let _ = event_sender.send(RqbitTorrentEvent::Failed(format!(
                                        "resume failed: {err}"
                                    )));
                                    return;
                                }
                            }
                        }
                        Ok(RqbitTorrentCommand::Remove) => {
                            match api.api_torrent_action_forget(torrent_id.into()).await {
                                Ok(_) => {
                                    let _ = event_sender.send(RqbitTorrentEvent::Removed);
                                    return;
                                }
                                Err(err) => {
                                    let _ = event_sender.send(RqbitTorrentEvent::Failed(format!(
                                        "remove failed: {err}"
                                    )));
                                    return;
                                }
                            }
                        }
                        Ok(RqbitTorrentCommand::DeleteFiles) => {
                            match api.api_torrent_action_delete(torrent_id.into()).await {
                                Ok(_) => {
                                    let _ = event_sender.send(RqbitTorrentEvent::Deleted);
                                    return;
                                }
                                Err(err) => {
                                    let _ = event_sender.send(RqbitTorrentEvent::Failed(format!(
                                        "delete failed: {err}"
                                    )));
                                    return;
                                }
                            }
                        }
                        Err(TryRecvError::Disconnected) => return,
                        Err(TryRecvError::Empty) => {}
                    }

                    match api.api_stats_v1(torrent_id.into()) {
                        Ok(stats) => {
                            let peers = api
                                .api_peer_stats(torrent_id.into(), Default::default())
                                .map(|snapshot| snapshot.peers.len() as u32)
                                .unwrap_or(0);
                            let download_rate_mbps = stats
                                .live
                                .as_ref()
                                .map(|live| live.download_speed.mbps as f32)
                                .unwrap_or(0.0);
                            let upload_rate_mbps = stats
                                .live
                                .as_ref()
                                .map(|live| live.upload_speed.mbps as f32)
                                .unwrap_or(0.0);
                            let eta_text = stats
                                .live
                                .as_ref()
                                .and_then(|live| live.time_remaining.as_ref())
                                .map(|eta| eta.to_string())
                                .unwrap_or_else(|| stats.state.to_string());
                            let progress_percent = if stats.total_bytes == 0 {
                                0.0
                            } else {
                                (stats.progress_bytes as f32 / stats.total_bytes as f32) * 100.0
                            };

                            let _ = event_sender.send(RqbitTorrentEvent::Progress {
                                progress_percent,
                                downloaded_bytes: stats.progress_bytes,
                                total_bytes: stats.total_bytes,
                                download_rate_mbps,
                                upload_rate_mbps,
                                peers,
                                eta_text: if is_paused {
                                    "Paused".to_owned()
                                } else {
                                    eta_text
                                },
                            });

                            if stats.finished {
                                let _ = event_sender.send(RqbitTorrentEvent::Completed);
                                return;
                            }
                        }
                        Err(err) => {
                            let _ = event_sender.send(RqbitTorrentEvent::Failed(format!(
                                "stats polling failed: {err}"
                            )));
                            return;
                        }
                    }

                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
        });

        ActiveRqbitTorrent {
            events: event_receiver,
            commands: command_sender,
        }
    }
}

fn session_options(privacy: &PrivacySettings) -> SessionOptions {
    SessionOptions {
        disable_dht: privacy.disable_peer_discovery_extras(),
        disable_dht_persistence: privacy.disable_peer_discovery_extras(),
        enable_upnp_port_forwarding: !privacy.disable_peer_discovery_extras(),
        ..Default::default()
    }
}
