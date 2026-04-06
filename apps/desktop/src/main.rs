use std::fs;
use std::path::{Path, PathBuf};

#[cfg(feature = "torrent-rqbit")]
use download_core::torrent_rqbit::{
    ActiveRqbitTorrent, RqbitTorrentCommand, RqbitTorrentEngine, RqbitTorrentEvent,
};
use download_core::{
    QueueManager, active_count,
    direct::{
        ActiveDirectDownload, DirectDownloadCommand, DirectDownloadEvent,
        build_direct_download_plan, create_resume_metadata, load_resume_metadata,
        spawn_direct_download,
    },
    ipc::{BrowserBridge, start_browser_bridge},
    load_snapshot_or_sample,
    planner::plan_download,
    save_snapshot,
    torrent::{
        TorrentTaskPlan, build_torrent_task_plan, create_torrent_session_snapshot,
        simulate_torrent_progress,
    },
    total_downloaded_mb,
};
use eframe::egui;
use egui::IconData;
#[cfg(feature = "torrent-rqbit")]
use shared::RqbitPersistedState;
#[cfg(feature = "torrent-rqbit")]
use shared::TorrentFileEntry;
use shared::{
    BrowserCapturePayload, DesktopPersistedState, DownloadKind, DownloadRecord, DownloadRequest,
    DownloadStatus, PrivacySettings, QueueView, TorrentSessionSnapshot,
};
#[cfg(windows)]
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

const BACKGROUND: egui::Color32 = egui::Color32::from_rgb(11, 15, 24);
const PANEL: egui::Color32 = egui::Color32::from_rgb(20, 28, 42);
const PANEL_ALT: egui::Color32 = egui::Color32::from_rgb(28, 38, 56);
const PANEL_HIGHLIGHT: egui::Color32 = egui::Color32::from_rgb(40, 56, 82);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(84, 196, 255);
const ACCENT_WARM: egui::Color32 = egui::Color32::from_rgb(255, 170, 76);
const SUCCESS: egui::Color32 = egui::Color32::from_rgb(94, 214, 143);
const DANGER: egui::Color32 = egui::Color32::from_rgb(237, 108, 88);
const MUTED_TEXT: egui::Color32 = egui::Color32::from_rgb(154, 168, 189);
const BRIGHT_TEXT: egui::Color32 = egui::Color32::from_rgb(236, 241, 248);

fn main() -> eframe::Result<()> {
    let launch_request = std::env::args().skip(1).find_map(parse_launch_request);
    let viewport = match load_app_icon() {
        Some(icon) => egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("NebulaDM")
            .with_icon(icon),
        None => egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("NebulaDM"),
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "NebulaDM",
        options,
        Box::new(move |_cc| Ok(Box::new(DesktopApp::new(launch_request.clone())))),
    )
}

fn load_app_icon() -> Option<IconData> {
    let bytes = include_bytes!("../../../assets/nebuladm-logo.png");
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (width, height) = image.dimensions();

    Some(IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

const APP_DIR_NAME: &str = "NebulaDM";
const APP_DATA_FALLBACK_DIR: &str = "data";
const DOWNLOAD_ROOT_FALLBACK_DIR: &str = "Downloads";

fn resolve_app_state_dir() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .map(|base| base.join(APP_DIR_NAME))
        .unwrap_or_else(|| PathBuf::from(APP_DATA_FALLBACK_DIR))
}

fn resolve_download_root() -> PathBuf {
    dirs::download_dir()
        .unwrap_or_else(|| PathBuf::from(DOWNLOAD_ROOT_FALLBACK_DIR))
        .join(APP_DIR_NAME)
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn infer_download_kind(source: &str) -> DownloadKind {
    let normalized = source.trim().to_ascii_lowercase();
    if normalized.starts_with("magnet:") || normalized.ends_with(".torrent") {
        DownloadKind::Torrent
    } else {
        DownloadKind::Direct
    }
}

fn parse_launch_request(argument: String) -> Option<DownloadRequest> {
    let trimmed = argument.trim();
    if !trimmed.to_ascii_lowercase().starts_with("magnet:") {
        return None;
    }

    let file_name =
        infer_magnet_display_name(trimmed).unwrap_or_else(|| "magnet-download.torrent".to_owned());
    Some(DownloadRequest::new(
        file_name,
        trimmed.to_owned(),
        DownloadKind::Torrent,
    ))
}

fn infer_magnet_display_name(magnet_uri: &str) -> Option<String> {
    let query = magnet_uri.split_once('?')?.1;
    for part in query.split('&') {
        if let Some(value) = part.strip_prefix("dn=") {
            let decoded = value.replace('+', " ");
            if !decoded.trim().is_empty() {
                return Some(decoded);
            }
        }
    }

    for part in query.split('&') {
        if let Some(value) = part.strip_prefix("xt=urn:btih:") {
            let short_hash: String = value.chars().take(12).collect();
            if !short_hash.is_empty() {
                return Some(format!("magnet-{short_hash}.torrent"));
            }
        }
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DownloadTab {
    Direct,
    Torrent,
}

impl DownloadTab {
    const ALL: [DownloadTab; 2] = [DownloadTab::Direct, DownloadTab::Torrent];

    fn label(self) -> &'static str {
        match self {
            Self::Direct => "Direct Downloads",
            Self::Torrent => "Torrent Downloads",
        }
    }

    fn kind(self) -> DownloadKind {
        match self {
            Self::Direct => DownloadKind::Direct,
            Self::Torrent => DownloadKind::Torrent,
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Direct => "Manage HTTP and HTTPS file transfers with resume support and browser handoff.",
            Self::Torrent => "Manage magnet links and torrent sessions with privacy-aware controls.",
        }
    }
}

struct DesktopApp {
    selected_tab: DownloadTab,
    queue_manager: QueueManager,
    selected_view: QueueView,
    storage_path: PathBuf,
    desktop_state_path: PathBuf,
    privacy_settings: PrivacySettings,
    new_name: String,
    new_source: String,
    new_kind: DownloadKind,
    status_message: String,
    pending_delete_history_confirmation: bool,
    pending_delete_confirmation_job_id: Option<u64>,
    expanded_details_job_id: Option<u64>,
    pending_browser_capture: Option<BrowserCapturePayload>,
    pending_launch_start_job_id: Option<u64>,
    active_direct_job_id: Option<u64>,
    active_direct_download: Option<ActiveDirectDownload>,
    browser_bridge: Option<BrowserBridge>,
    browser_bridge_addr: String,
    active_torrent_job_id: Option<u64>,
    active_torrent_plan: Option<TorrentTaskPlan>,
    active_torrent_session: Option<TorrentSessionSnapshot>,
    #[cfg(feature = "torrent-rqbit")]
    rqbit_engine: RqbitTorrentEngine,
    #[cfg(feature = "torrent-rqbit")]
    rqbit_torrent: Option<ActiveRqbitTorrent>,
    #[cfg(feature = "torrent-rqbit")]
    rqbit_torrent_name: Option<String>,
    #[cfg(feature = "torrent-rqbit")]
    rqbit_info_hash: Option<String>,
    #[cfg(feature = "torrent-rqbit")]
    rqbit_output_folder: Option<String>,
    #[cfg(feature = "torrent-rqbit")]
    rqbit_file_count: Option<usize>,
    #[cfg(feature = "torrent-rqbit")]
    rqbit_files: Vec<TorrentFileEntry>,
    #[cfg(feature = "torrent-rqbit")]
    rqbit_peer_count: u32,
    #[cfg(feature = "torrent-rqbit")]
    pending_rqbit_restore: Option<RqbitPersistedState>,
}

impl DesktopApp {
    fn new(launch_request: Option<DownloadRequest>) -> Self {
        let app_state_dir = resolve_app_state_dir();
        let download_root = resolve_download_root();
        let storage_path = app_state_dir.join("snapshot.json");
        let desktop_state_path = app_state_dir.join("desktop-state.json");
        let snapshot = load_snapshot_or_sample(&storage_path, &display_path(&download_root));
        let desktop_state = DesktopPersistedState::load(&desktop_state_path);
        let mut queue_manager = QueueManager::new(snapshot);

        let mut selected_view = QueueView::Active;
        let mut selected_tab = DownloadTab::Direct;
        let mut status_message = "Snapshot loaded".to_owned();
        let mut pending_launch_start_job_id = None;
        if let Some(request) = launch_request {
            let id = queue_manager.add_download_request(request, false);
            selected_view = QueueView::Torrents;
            selected_tab = DownloadTab::Torrent;
            status_message = format!("Opened magnet link as job #{id}");
            pending_launch_start_job_id = Some(id);
        }

        let browser_bridge_addr = "127.0.0.1:35791".to_owned();
        let browser_bridge = start_browser_bridge(&browser_bridge_addr).ok();
        let auto_register_status = auto_register_magnet_protocol();
        let status_message = match auto_register_status {
            Some(message) => format!("{status_message} | {message}"),
            None => status_message,
        };

        Self {
            selected_tab,
            queue_manager,
            selected_view,
            storage_path,
            desktop_state_path,
            privacy_settings: desktop_state.privacy.clone(),
            new_name: "fedora-workstation.iso".to_owned(),
            new_source: "https://download.fedoraproject.org/pub/fedora.iso".to_owned(),
            new_kind: DownloadKind::Direct,
            status_message,
            pending_delete_history_confirmation: false,
            pending_delete_confirmation_job_id: None,
            expanded_details_job_id: {
                #[cfg(feature = "torrent-rqbit")]
                {
                    desktop_state.expanded_details_job_id
                }
                #[cfg(not(feature = "torrent-rqbit"))]
                {
                    None
                }
            },
            pending_browser_capture: None,
            pending_launch_start_job_id,
            active_direct_job_id: None,
            active_direct_download: None,
            browser_bridge,
            browser_bridge_addr,
            active_torrent_job_id: None,
            active_torrent_plan: None,
            active_torrent_session: None,
            #[cfg(feature = "torrent-rqbit")]
            rqbit_engine: RqbitTorrentEngine::new(
                download_root.join("Torrents"),
                desktop_state.privacy.clone(),
            ),
            #[cfg(feature = "torrent-rqbit")]
            rqbit_torrent: None,
            #[cfg(feature = "torrent-rqbit")]
            rqbit_torrent_name: desktop_state
                .rqbit
                .as_ref()
                .and_then(|state| state.torrent_name.clone()),
            #[cfg(feature = "torrent-rqbit")]
            rqbit_info_hash: desktop_state
                .rqbit
                .as_ref()
                .and_then(|state| state.info_hash.clone()),
            #[cfg(feature = "torrent-rqbit")]
            rqbit_output_folder: desktop_state
                .rqbit
                .as_ref()
                .and_then(|state| state.output_folder.clone()),
            #[cfg(feature = "torrent-rqbit")]
            rqbit_file_count: desktop_state
                .rqbit
                .as_ref()
                .and_then(|state| state.file_count),
            #[cfg(feature = "torrent-rqbit")]
            rqbit_files: desktop_state
                .rqbit
                .as_ref()
                .map(|state| state.files.clone())
                .unwrap_or_default(),
            #[cfg(feature = "torrent-rqbit")]
            rqbit_peer_count: desktop_state
                .rqbit
                .as_ref()
                .map(|state| state.peer_count)
                .unwrap_or(0),
            #[cfg(feature = "torrent-rqbit")]
            pending_rqbit_restore: desktop_state.rqbit,
        }
    }
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_theme(ctx);
        self.handle_launch_request_if_needed();
        #[cfg(feature = "torrent-rqbit")]
        self.restore_rqbit_job_if_needed();
        self.poll_direct_events();
        self.poll_browser_bridge();
        #[cfg(feature = "torrent-rqbit")]
        self.poll_rqbit_torrent_events();
        self.tick_torrent_session();
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
        let snapshot = self.queue_manager.snapshot().clone();
        let visible_records: Vec<DownloadRecord> = self
            .queue_manager
            .snapshot()
            .queue
            .iter()
            .cloned()
            .filter(|item| {
                item.request.kind == self.selected_tab.kind()
                    && item.is_visible_in(self.selected_view)
            })
            .collect();

        egui::TopBottomPanel::top("top_bar")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(20, 18)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("NebulaDM")
                                .size(28.0)
                                .strong()
                                .color(BRIGHT_TEXT),
                        );
                        ui.label(
                            egui::RichText::new(
                                "Fast direct downloads, integrated torrents, and browser capture in one desktop app",
                            )
                            .color(MUTED_TEXT),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        let bridge_badge = egui::Frame::new()
                            .fill(PANEL_HIGHLIGHT)
                            .corner_radius(12.0)
                            .inner_margin(egui::Margin::symmetric(12, 8));
                        bridge_badge.show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(format!("Bridge: {}", self.browser_bridge_addr))
                                    .color(ACCENT),
                            );
                        });
                    });
                });
        });

        egui::SidePanel::left("sidebar")
            .min_width(210.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(18, 18)),
            )
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Queues")
                        .size(20.0)
                        .strong()
                        .color(BRIGHT_TEXT),
                );
                ui.add_space(8.0);
                for view in QueueView::ALL {
                    let count = self
                        .queue_manager
                        .snapshot()
                        .queue
                        .iter()
                        .filter(|item| item.is_visible_in(view))
                        .count();
                    let selected = self.selected_view == view;
                    let label = format!("{} ({count})", view.label());
                    let button = egui::Button::new(egui::RichText::new(label).color(if selected {
                        BRIGHT_TEXT
                    } else {
                        MUTED_TEXT
                    }))
                    .fill(if selected { PANEL_HIGHLIGHT } else { PANEL_ALT })
                    .corner_radius(12.0)
                    .min_size(egui::vec2(ui.available_width(), 36.0));

                    if ui.add(button).clicked() {
                        self.selected_view = view;
                    }
                    ui.add_space(6.0);
                }

                ui.add_space(16.0);
                ui.label(
                    egui::RichText::new("Status")
                        .size(18.0)
                        .strong()
                        .color(BRIGHT_TEXT),
                );
                ui.label(
                    egui::RichText::new(format!("Active: {}", active_count(&snapshot)))
                        .color(MUTED_TEXT),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "Downloaded: {:.1} MB",
                        total_downloaded_mb(&snapshot)
                    ))
                    .color(MUTED_TEXT),
                );
                ui.label(
                    egui::RichText::new(format!("Engine: {}", self.torrent_engine_label()))
                        .color(MUTED_TEXT),
                );
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("Privacy")
                        .size(18.0)
                        .strong()
                        .color(BRIGHT_TEXT),
                );
                let mut privacy_mode = self.privacy_settings.privacy_mode;
                if ui.checkbox(&mut privacy_mode, "Privacy Mode").changed() {
                    self.set_privacy_mode(privacy_mode);
                }
                ui.label(
                    egui::RichText::new(
                        "Enforces auto-stop, no seeding, minimal metadata retention, and quieter local state.",
                    )
                    .color(MUTED_TEXT),
                );
                let delete_label = if self.pending_delete_history_confirmation {
                    "Confirm Delete History + Metadata"
                } else {
                    "Delete History + Metadata"
                };
                if ui.button(delete_label).clicked() {
                    self.request_delete_history_and_metadata();
                }
                if ui.button("Setup Browser Extension").clicked() {
                    self.open_browser_extension_setup();
                }
                if ui.button("Register Magnet Links").clicked() {
                    self.register_magnet_protocol();
                }
                #[cfg(feature = "torrent-rqbit")]
                if let Some(name) = &self.rqbit_torrent_name {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Live Torrent")
                            .color(MUTED_TEXT)
                            .strong(),
                    );
                    ui.label(egui::RichText::new(name).color(BRIGHT_TEXT));
                    ui.label(
                        egui::RichText::new(format!("Peers: {}", self.rqbit_peer_count))
                            .color(MUTED_TEXT),
                    );
                }
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(18, 18)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for tab in DownloadTab::ALL {
                        let selected = self.selected_tab == tab;
                        let button = egui::Button::new(
                            egui::RichText::new(tab.label()).color(if selected {
                                BRIGHT_TEXT
                            } else {
                                MUTED_TEXT
                            }),
                        )
                        .fill(if selected { PANEL_HIGHLIGHT } else { PANEL_ALT })
                        .corner_radius(14.0)
                        .min_size(egui::vec2(190.0, 40.0));

                        if ui.add(button).clicked() {
                            self.selected_tab = tab;
                            self.new_kind = tab.kind();
                        }
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} | {}",
                                self.selected_tab.label(),
                                self.selected_view.label()
                            ))
                                .size(24.0)
                                .strong()
                                .color(BRIGHT_TEXT),
                        );
                        ui.label(
                            egui::RichText::new(self.selected_tab.subtitle()).color(MUTED_TEXT),
                        );
                    });
                });
                self.render_quick_add_panel(ui, &snapshot);

            ui.add_space(14.0);

            if visible_records.is_empty() {
                egui::Frame::new()
                    .fill(PANEL)
                    .corner_radius(18.0)
                    .inner_margin(egui::Margin::symmetric(18, 18))
                    .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("No items in this view")
                            .size(20.0)
                            .strong()
                            .color(BRIGHT_TEXT),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "Matching {} items will appear here when they enter the {} queue.",
                            self.selected_tab.label().to_ascii_lowercase(),
                            self.selected_view.label().to_ascii_lowercase()
                        ))
                            .color(MUTED_TEXT),
                    );
                });
            } else {
                for record in visible_records {
                    let plan = plan_download(
                        &record.request,
                        &snapshot.downloads_root,
                        &snapshot.categories,
                    );

                    let row_response = egui::Frame::new()
                        .fill(PANEL)
                        .stroke(egui::Stroke::new(1.0, PANEL_HIGHLIGHT))
                        .corner_radius(18.0)
                        .inner_margin(egui::Margin::symmetric(18, 16))
                        .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(&record.request.file_name)
                                        .size(20.0)
                                        .strong()
                                        .color(BRIGHT_TEXT),
                                );
                                ui.horizontal(|ui| {
                                    self.pill(
                                        ui,
                                        &record.request.kind.to_string(),
                                        match record.request.kind {
                                            DownloadKind::Direct => ACCENT,
                                            DownloadKind::Torrent => ACCENT_WARM,
                                        },
                                    );
                                    self.pill(
                                        ui,
                                        &record.status.to_string(),
                                        self.status_color(&record.status.to_string()),
                                    );
                                });
                            });
                            ui.add_space(10.0);
                            let details_label = if self.expanded_details_job_id == Some(record.id) {
                                "Hide Details"
                            } else {
                                "Details"
                            };
                            if ui.button(details_label).clicked() {
                                self.toggle_details_for(record.id);
                            }
                            if record.request.kind == DownloadKind::Torrent {
                            }
                        });
                        ui.label(egui::RichText::new(&record.request.source).color(MUTED_TEXT));
                        ui.add(
                            egui::ProgressBar::new(record.progress_percent / 100.0)
                                .desired_width(ui.available_width())
                                .fill(ACCENT)
                                .text(format!("{:.1}%", record.progress_percent)),
                        );
                        ui.horizontal(|ui| {
                            ui.monospace(format!(
                                "{:.1}/{:.1} MB",
                                record.downloaded_mb, record.total_mb
                            ));
                            ui.monospace(format!("Speed: {:.1} MB/s", record.speed_mbps));
                            ui.monospace(format!("ETA: {}", record.eta_text));
                        });
                        ui.horizontal(|ui| {
                            let can_pause = matches!(
                                record.status,
                                DownloadStatus::Downloading
                            );
                            let can_resume = matches!(
                                record.status,
                                DownloadStatus::Paused | DownloadStatus::Queued | DownloadStatus::Failed
                            );
                            if ui
                                .add_enabled(can_pause, egui::Button::new("Pause"))
                                .clicked()
                            {
                                self.pending_delete_confirmation_job_id = None;
                                self.request_pause_for(record.id);
                            }
                            if ui
                                .add_enabled(can_resume, egui::Button::new("Resume"))
                                .clicked()
                            {
                                self.pending_delete_confirmation_job_id = None;
                                self.request_resume_for(record.id);
                            }
                        });
                        if self.pending_delete_confirmation_job_id == Some(record.id) {
                            ui.colored_label(
                                DANGER,
                                "Warning: Confirm Delete will permanently remove this torrent's downloaded files from disk.",
                            );
                        }
                        if self.expanded_details_job_id == Some(record.id) {
                            match record.request.kind {
                                DownloadKind::Torrent => self.render_torrent_details(ui, &record, &plan),
                                DownloadKind::Direct => self.render_direct_details(ui, &record),
                            }
                        }
                        ui.label(egui::RichText::new(format!("Category: {}", plan.category_name)).color(MUTED_TEXT));
                        ui.label(egui::RichText::new(format!("Target Folder: {}", plan.target_folder)).color(MUTED_TEXT));
                    });
                    row_response.response.context_menu(|ui| {
                        self.render_transfer_context_menu(ui, &record);
                    });

                    ui.add_space(10.0);
                }
            }

        });

        self.render_browser_capture_confirmation(ctx);
    }
}

impl DesktopApp {
    fn render_transfer_context_menu(&mut self, ui: &mut egui::Ui, record: &DownloadRecord) {
        ui.label(
            egui::RichText::new(&record.request.file_name)
                .color(BRIGHT_TEXT)
                .strong(),
        );
        ui.label(
            egui::RichText::new(format!("{} | {}", record.request.kind, record.status))
                .color(MUTED_TEXT),
        );
        ui.separator();

        let can_pause = matches!(
            record.status,
            DownloadStatus::Downloading | DownloadStatus::Seeding
        );
        let can_resume = matches!(
            record.status,
            DownloadStatus::Paused | DownloadStatus::Queued | DownloadStatus::Failed
        );

        if ui.button("Details").clicked() {
            self.toggle_details_for(record.id);
            ui.close();
        }
        if ui
            .add_enabled(can_pause, egui::Button::new("Pause"))
            .clicked()
        {
            self.pending_delete_confirmation_job_id = None;
            self.request_pause_for(record.id);
            ui.close();
        }
        if ui
            .add_enabled(can_resume, egui::Button::new("Resume"))
            .clicked()
        {
            self.pending_delete_confirmation_job_id = None;
            self.request_resume_for(record.id);
            ui.close();
        }
        if ui.button("Remove").clicked() {
            self.pending_delete_confirmation_job_id = None;
            self.request_remove_for(record.id);
            ui.close();
        }
        let delete_label = if self.pending_delete_confirmation_job_id == Some(record.id) {
            "Confirm Delete"
        } else {
            "Delete Files"
        };
        if ui.button(delete_label).clicked() {
            self.request_delete_files_for(record.id);
            ui.close();
        }
    }

    fn render_quick_add_panel(&mut self, ui: &mut egui::Ui, snapshot: &shared::AppSnapshot) {
        egui::Frame::new()
            .fill(PANEL)
            .corner_radius(18.0)
            .inner_margin(egui::Margin::symmetric(18, 18))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Quick Add")
                        .size(20.0)
                        .strong()
                        .color(BRIGHT_TEXT),
                );
                ui.label(
                    egui::RichText::new(
                        "IDM-style link capture with BitTorrent-ready routing for magnet links and .torrent URLs.",
                    )
                    .color(MUTED_TEXT),
                );
                ui.add_space(10.0);
                ui.label(egui::RichText::new("File Name").color(MUTED_TEXT));
                ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text("ubuntu.iso or movie.torrent"));
                ui.label(egui::RichText::new("Source URL or Magnet").color(MUTED_TEXT));
                let source_response = ui.add(
                    egui::TextEdit::singleline(&mut self.new_source)
                        .hint_text("https://example.com/file.zip or magnet:?xt=..."),
                );
                if source_response.changed() {
                    self.new_kind = infer_download_kind(&self.new_source);
                }
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.new_kind, DownloadKind::Direct, "Direct");
                    ui.selectable_value(&mut self.new_kind, DownloadKind::Torrent, "Torrent");
                    if ui.button("Add To Queue").clicked() {
                        self.add_manual_download();
                    }
                    if ui.button("Start Direct").clicked() {
                        self.start_next_direct_download();
                    }
                    if ui.button("Start Torrent").clicked() {
                        self.start_next_torrent_download();
                    }
                });
                ui.add_space(10.0);
                ui.label(egui::RichText::new(format!("Status: {}", self.status_message)).color(ACCENT).strong());
                ui.label(egui::RichText::new(self.quick_add_hint()).color(ACCENT_WARM).strong());
                if self.new_kind == DownloadKind::Direct {
                    let preview_request = shared::DownloadRequest::new(
                        self.new_name.trim().to_owned(),
                        self.new_source.trim().to_owned(),
                        DownloadKind::Direct,
                    );
                    let direct_plan = build_direct_download_plan(
                        &preview_request,
                        &snapshot.downloads_root,
                        &snapshot.categories,
                    );
                    let metadata = create_resume_metadata(&preview_request, &direct_plan, None);
                    ui.monospace(format!("Target: {}", direct_plan.final_file_path));
                    ui.monospace(format!("Chunks: {}", metadata.chunks.len()));
                } else {
                    let preview_request = shared::DownloadRequest::new(
                        self.new_name.trim().to_owned(),
                        self.new_source.trim().to_owned(),
                        DownloadKind::Torrent,
                    );
                    let torrent_plan = build_torrent_task_plan(
                        &preview_request,
                        &snapshot.downloads_root,
                        &snapshot.categories,
                    );
                    ui.monospace(format!("Torrent Root: {}", torrent_plan.data_root));
                    ui.monospace(format!("Session: {}", torrent_plan.session_file_path));
                }
            });
    }
    fn render_browser_capture_confirmation(&mut self, ctx: &egui::Context) {
        let Some(payload) = self.pending_browser_capture.clone() else {
            return;
        };

        egui::Window::new("Confirm Browser Download")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .collapsible(false)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(egui::Stroke::new(1.0, PANEL_HIGHLIGHT))
                    .corner_radius(18.0)
                    .inner_margin(egui::Margin::symmetric(18, 18)),
            )
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("NebulaDM received a browser download request.")
                        .color(BRIGHT_TEXT)
                        .size(20.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(
                        "Continue this download in NebulaDM, or let the browser keep it?",
                    )
                    .color(MUTED_TEXT),
                );
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(format!("File: {}", payload.file_name))
                        .color(BRIGHT_TEXT)
                        .strong(),
                );
                ui.label(egui::RichText::new(format!("Type: {}", payload.kind)).color(MUTED_TEXT));
                ui.label(
                    egui::RichText::new(format!("Source: {}", payload.source)).color(MUTED_TEXT),
                );
                if let Some(referrer) = &payload.referrer
                    && !self.privacy_settings.minimize_browser_metadata_retention()
                {
                    ui.label(
                        egui::RichText::new(format!("Referrer: {referrer}")).color(MUTED_TEXT),
                    );
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new("Download In NebulaDM").strong())
                                .fill(ACCENT)
                                .corner_radius(12.0),
                        )
                        .clicked()
                    {
                        self.accept_pending_browser_capture();
                    }

                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new("Dismiss").color(MUTED_TEXT))
                                .fill(PANEL_ALT)
                                .corner_radius(12.0),
                        )
                        .clicked()
                    {
                        self.reject_pending_browser_capture();
                    }
                });
            });
    }

    fn accept_pending_browser_capture(&mut self) {
        let Some(payload) = self.pending_browser_capture.take() else {
            return;
        };

        let mut request = payload.into_request();
        if self.privacy_settings.minimize_browser_metadata_retention() {
            request.clear_browser_context();
        }
        let id = self.queue_manager.add_download_request(request, true);
        self.selected_view = QueueView::BrowserCapture;
        self.status_message = format!("Accepted browser download into job #{id}");
        let _ = save_snapshot(&self.storage_path, self.queue_manager.snapshot());
        self.save_desktop_state();
    }

    fn reject_pending_browser_capture(&mut self) {
        if let Some(payload) = self.pending_browser_capture.take() {
            self.status_message = format!("Dismissed browser capture for {}", payload.file_name);
        }
    }

    fn set_privacy_mode(&mut self, enabled: bool) {
        self.privacy_settings.privacy_mode = enabled;
        #[cfg(feature = "torrent-rqbit")]
        {
            let download_root = resolve_download_root();
            self.rqbit_engine = RqbitTorrentEngine::new(
                download_root.join("Torrents"),
                self.privacy_settings.clone(),
            );
        }
        self.status_message = if enabled {
            "Privacy Mode enabled".to_owned()
        } else {
            "Privacy Mode disabled".to_owned()
        };
        self.save_desktop_state();
        let _ = save_snapshot(&self.storage_path, self.queue_manager.snapshot());
    }

    fn has_active_transfers(&self) -> bool {
        self.active_direct_job_id.is_some()
            || self.active_torrent_job_id.is_some()
            || self.queue_manager.snapshot().queue.iter().any(|item| {
                matches!(item.status, DownloadStatus::Downloading | DownloadStatus::Paused)
            })
    }

    fn request_delete_history_and_metadata(&mut self) {
        if !self.pending_delete_history_confirmation {
            self.pending_delete_history_confirmation = true;
            self.status_message = "Click again to permanently delete history and saved metadata"
                .to_owned();
            return;
        }

        self.pending_delete_history_confirmation = false;

        if self.has_active_transfers() {
            self.status_message =
                "Pause or remove active transfers before deleting history and metadata".to_owned();
            return;
        }

        let snapshot = self.queue_manager.snapshot().clone();
        for record in &snapshot.queue {
            match record.request.kind {
                DownloadKind::Direct => {
                    let plan = build_direct_download_plan(
                        &record.request,
                        &snapshot.downloads_root,
                        &snapshot.categories,
                    );
                    let _ = fs::remove_file(&plan.metadata_file_path);
                }
                DownloadKind::Torrent => {
                    let plan = build_torrent_task_plan(
                        &record.request,
                        &snapshot.downloads_root,
                        &snapshot.categories,
                    );
                    let _ = fs::remove_file(&plan.session_file_path);
                }
            }
        }

        self.queue_manager.clear_all_history();
        self.pending_browser_capture = None;
        self.expanded_details_job_id = None;
        self.pending_delete_confirmation_job_id = None;
        self.active_direct_job_id = None;
        self.active_direct_download = None;
        self.active_torrent_job_id = None;
        self.active_torrent_plan = None;
        self.active_torrent_session = None;
        #[cfg(feature = "torrent-rqbit")]
        {
            self.rqbit_torrent = None;
            self.rqbit_torrent_name = None;
            self.rqbit_info_hash = None;
            self.rqbit_output_folder = None;
            self.rqbit_file_count = None;
            self.rqbit_files.clear();
            self.rqbit_peer_count = 0;
            self.pending_rqbit_restore = None;
        }

        let _ = fs::remove_file(&self.storage_path);
        let _ = fs::remove_file(&self.desktop_state_path);
        let _ = save_snapshot(&self.storage_path, self.queue_manager.snapshot());
        self.save_desktop_state();
        self.status_message = "Deleted history and saved metadata".to_owned();
    }

    fn handle_launch_request_if_needed(&mut self) {
        let Some(id) = self.pending_launch_start_job_id.take() else {
            return;
        };

        self.start_torrent_download_for(id);
    }

    fn add_manual_download(&mut self) {
        let file_name = self.new_name.trim();
        let source = self.new_source.trim();

        if file_name.is_empty() || source.is_empty() {
            self.status_message =
                "Enter both a file name and a source URL or magnet link".to_owned();
            return;
        }

        let detected_kind = infer_download_kind(source);
        if detected_kind != self.new_kind {
            self.new_kind = detected_kind.clone();
        }

        let id = self.queue_manager.add_download(
            file_name.to_owned(),
            source.to_owned(),
            detected_kind,
            false,
        );
        self.status_message = format!("Added job #{id} to the queue");
        let _ = save_snapshot(&self.storage_path, self.queue_manager.snapshot());
        self.save_desktop_state();
    }

    fn quick_add_hint(&self) -> &'static str {
        match self.new_kind {
            DownloadKind::Direct => {
                "Direct mode: use standard HTTP/HTTPS file links. Magnet links switch to Torrent automatically."
            }
            DownloadKind::Torrent => {
                "Torrent mode: use a magnet link or a .torrent URL. The desktop app will save it under Downloads/NebulaDM/Torrents."
            }
        }
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(12.0, 8.0);
        style.visuals = egui::Visuals::dark();
        style.visuals.override_text_color = Some(BRIGHT_TEXT);
        style.visuals.widgets.noninteractive.bg_fill = PANEL;
        style.visuals.widgets.inactive.bg_fill = PANEL_ALT;
        style.visuals.widgets.hovered.bg_fill = PANEL_HIGHLIGHT;
        style.visuals.widgets.active.bg_fill = PANEL_HIGHLIGHT;
        style.visuals.widgets.open.bg_fill = PANEL_HIGHLIGHT;
        style.visuals.window_fill = BACKGROUND;
        style.visuals.panel_fill = BACKGROUND;
        style.visuals.faint_bg_color = PANEL_ALT;
        style.visuals.hyperlink_color = ACCENT;
        style.visuals.selection.bg_fill = ACCENT.linear_multiply(0.35);
        style.visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
        style.visuals.widgets.inactive.corner_radius = 12.0.into();
        style.visuals.widgets.hovered.corner_radius = 12.0.into();
        style.visuals.widgets.active.corner_radius = 12.0.into();
        ctx.set_style(style);
    }

    fn pill(&self, ui: &mut egui::Ui, text: &str, tint: egui::Color32) {
        egui::Frame::new()
            .fill(tint.linear_multiply(0.22))
            .stroke(egui::Stroke::new(1.0, tint))
            .corner_radius(999.0)
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.label(egui::RichText::new(text).color(tint).strong());
            });
    }

    fn status_color(&self, status: &str) -> egui::Color32 {
        let lowercase = status.to_ascii_lowercase();
        if lowercase.contains("completed") {
            SUCCESS
        } else if lowercase.contains("failed") || lowercase.contains("delete") {
            DANGER
        } else if lowercase.contains("paused") {
            ACCENT_WARM
        } else {
            ACCENT
        }
    }

    fn register_magnet_protocol(&mut self) {
        #[cfg(windows)]
        {
            match register_magnet_protocol_handler() {
                Ok(()) => {
                    self.status_message =
                        "NebulaDM is now registered as the magnet-link handler for this Windows user".to_owned();
                }
                Err(err) => {
                    self.status_message = format!("Magnet registration failed: {err}");
                }
            }
        }

        #[cfg(not(windows))]
        {
            self.status_message =
                "Magnet protocol registration is currently implemented only for Windows".to_owned();
        }
    }

    fn open_browser_extension_setup(&mut self) {
        match resolve_browser_extension_dir() {
            Some(path) => {
                #[cfg(windows)]
                {
                    match std::process::Command::new("explorer").arg(&path).spawn() {
                        Ok(_) => {
                            self.status_message = format!(
                                "Opened browser extension folder: {}",
                                path.display()
                            );
                        }
                        Err(err) => {
                            self.status_message =
                                format!("Could not open browser extension folder: {err}");
                        }
                    }
                }

                #[cfg(not(windows))]
                {
                    self.status_message =
                        format!("Browser extension folder is available at {}", path.display());
                }
            }
            None => {
                self.status_message =
                    "Browser extension folder was not found near the app or workspace".to_owned();
            }
        }
    }

    fn toggle_details_for(&mut self, id: u64) {
        if self.expanded_details_job_id == Some(id) {
            self.expanded_details_job_id = None;
        } else {
            self.expanded_details_job_id = Some(id);
        }
        self.save_desktop_state();
    }

    fn render_torrent_details(
        &self,
        ui: &mut egui::Ui,
        record: &DownloadRecord,
        plan: &shared::DownloadPlan,
    ) {
        ui.separator();
        ui.label("Torrent Details");
        ui.monospace(format!("Category Folder: {}", plan.target_folder));

        let is_active_simulated =
            self.active_torrent_job_id == Some(record.id) && self.active_torrent_session.is_some();
        if is_active_simulated {
            if let Some(session) = &self.active_torrent_session {
                ui.monospace(format!("Save Path: {}", session.save_path));
                ui.monospace(format!("Info Hash: {}", session.info_hash));
                ui.monospace(format!(
                    "Pieces: {}/{}",
                    session.completed_pieces, session.piece_count
                ));
                ui.monospace(format!("Peers: {}", session.connected_peers));
                if !self.privacy_settings.minimize_torrent_metadata_retention() {
                    for tracker in session.trackers.iter().take(3) {
                        ui.monospace(format!("Tracker: {tracker}"));
                    }
                    for file in session.files.iter().take(3) {
                        ui.monospace(format!(
                            "File: {} ({:.1} MB)",
                            file.path,
                            file.size_bytes as f32 / (1024.0 * 1024.0)
                        ));
                    }
                }
                return;
            }
        }

        #[cfg(feature = "torrent-rqbit")]
        if self.active_torrent_job_id == Some(record.id) && self.active_torrent_session.is_none() {
            if let Some(output_folder) = &self.rqbit_output_folder {
                ui.monospace(format!("Save Path: {output_folder}"));
            } else {
                ui.monospace(format!(
                    "Save Root: {}/{}",
                    plan.target_folder, record.request.file_name
                ));
            }
            if let Some(name) = &self.rqbit_torrent_name {
                ui.monospace(format!("Resolved Name: {name}"));
            }
            if let Some(info_hash) = &self.rqbit_info_hash {
                ui.monospace(format!("Info Hash: {info_hash}"));
            }
            if let Some(file_count) = self.rqbit_file_count {
                ui.monospace(format!("Files: {file_count}"));
            }
            ui.monospace(format!("Peers: {}", self.rqbit_peer_count));
            if !self.privacy_settings.minimize_torrent_metadata_retention() {
                ui.monospace(format!("Magnet: {}", record.request.source));
                for file in self.rqbit_files.iter().take(5) {
                    ui.monospace(format!(
                        "File: {} ({:.1} MB)",
                        file.path,
                        file.size_bytes as f32 / (1024.0 * 1024.0)
                    ));
                }
            }
            return;
        }

        let snapshot = self.queue_manager.snapshot();
        let torrent_plan = build_torrent_task_plan(
            &record.request,
            &snapshot.downloads_root,
            &snapshot.categories,
        );
        let session = create_torrent_session_snapshot(&torrent_plan);
        ui.monospace(format!("Save Path: {}", session.save_path));
        ui.monospace(format!("Info Hash: {}", session.info_hash));
        if !self.privacy_settings.minimize_torrent_metadata_retention() {
            ui.monospace(format!("Trackers: {}", session.trackers.len()));
            for tracker in session.trackers.iter().take(2) {
                ui.monospace(format!("Tracker: {tracker}"));
            }
            for file in session.files.iter().take(2) {
                ui.monospace(format!(
                    "File: {} ({:.1} MB)",
                    file.path,
                    file.size_bytes as f32 / (1024.0 * 1024.0)
                ));
            }
        }
    }

    fn render_direct_details(&self, ui: &mut egui::Ui, record: &DownloadRecord) {
        ui.separator();
        ui.label("Direct Details");

        let snapshot = self.queue_manager.snapshot();
        let plan = build_direct_download_plan(
            &record.request,
            &snapshot.downloads_root,
            &snapshot.categories,
        );
        ui.monospace(format!("Target File: {}", plan.final_file_path));
        ui.monospace(format!("Temp File: {}", plan.temp_file_path));
        ui.monospace(format!("Resume State: {}", plan.metadata_file_path));
        ui.monospace(format!(
            "Parallel Connections: {}",
            plan.parallel_connections
        ));
        ui.monospace(format!(
            "Chunk Size: {:.1} MB",
            plan.chunk_size_bytes as f32 / (1024.0 * 1024.0)
        ));
        if let Some(referrer) = &record.request.referrer
            && !self.privacy_settings.minimize_browser_metadata_retention()
        {
            ui.monospace(format!("Referrer: {referrer}"));
        }
        if let Some(user_agent) = &record.request.user_agent
            && !self.privacy_settings.minimize_browser_metadata_retention()
        {
            ui.monospace(format!("User-Agent: {user_agent}"));
        }
        if let Some(cookie_header) = &record.request.cookie_header
            && !self.privacy_settings.minimize_browser_metadata_retention()
        {
            ui.monospace(format!(
                "Cookies: {} entries",
                cookie_header.split(';').count()
            ));
        }

        if let Some(metadata) = load_resume_metadata(std::path::Path::new(&plan.metadata_file_path))
        {
            ui.monospace(format!("Tracked Chunks: {}", metadata.chunks.len()));
            for chunk in metadata.chunks.iter().take(6) {
                let chunk_total = chunk
                    .end_byte
                    .map(|end| end.saturating_sub(chunk.start_byte) + 1)
                    .unwrap_or(chunk.downloaded_bytes);
                let percent = if chunk_total == 0 {
                    0.0
                } else {
                    (chunk.downloaded_bytes as f32 / chunk_total as f32) * 100.0
                };
                ui.monospace(format!(
                    "Chunk {}: bytes {}-{} | {:.1}%",
                    chunk.index,
                    chunk.start_byte,
                    chunk
                        .end_byte
                        .unwrap_or(chunk.start_byte + chunk.downloaded_bytes),
                    percent.min(100.0)
                ));
            }
        } else {
            let preview_metadata = create_resume_metadata(&record.request, &plan, None);
            ui.monospace(format!("Planned Chunks: {}", preview_metadata.chunks.len()));
            ui.monospace("Chunk metadata will appear once the direct worker starts.");
        }
    }

    fn scrub_completed_direct_metadata(&mut self, job_id: u64) {
        if self.privacy_settings.minimize_browser_metadata_retention() {
            self.queue_manager.clear_browser_metadata(job_id);
        }
    }

    fn scrub_completed_torrent_metadata(&mut self, job_id: u64) {
        if self.privacy_settings.minimize_torrent_metadata_retention() {
            self.queue_manager.redact_torrent_source(job_id);
        }
    }

    #[cfg(feature = "torrent-rqbit")]
    fn clear_rqbit_session_metadata(&mut self) {
        self.rqbit_torrent = None;
        self.rqbit_torrent_name = None;
        self.rqbit_info_hash = None;
        self.rqbit_output_folder = None;
        self.rqbit_file_count = None;
        self.rqbit_files.clear();
        self.rqbit_peer_count = 0;
    }

    fn start_next_direct_download(&mut self) {
        if self.active_direct_download.is_some() {
            self.status_message = "A direct download worker is already active".to_owned();
            return;
        }

        let Some(id) = self.queue_manager.start_next_queued_direct() else {
            self.status_message = "No queued direct download found".to_owned();
            return;
        };

        self.start_direct_download_for(id);
    }

    fn start_direct_download_for(&mut self, id: u64) {
        if self.active_direct_download.is_some() {
            self.status_message = "A direct download worker is already active".to_owned();
            return;
        }

        let snapshot = self.queue_manager.snapshot().clone();
        let Some(record) = self.queue_manager.get_record(id).cloned() else {
            self.status_message = "Started job was not found in the queue".to_owned();
            return;
        };

        let plan = build_direct_download_plan(
            &record.request,
            &snapshot.downloads_root,
            &snapshot.categories,
        );
        self.queue_manager.resume(id);
        self.active_direct_download = Some(spawn_direct_download(record.request.clone(), plan));
        self.active_direct_job_id = Some(id);
        self.status_message = format!("Started direct job #{id}");
    }

    fn poll_direct_events(&mut self) {
        let Some(job_id) = self.active_direct_job_id else {
            return;
        };

        let mut finished = false;
        let mut scrub_completed_metadata = false;
        if let Some(active_download) = &self.active_direct_download {
            while let Ok(event) = active_download.events.try_recv() {
                match event {
                    DirectDownloadEvent::Started { total_bytes } => {
                        self.queue_manager.set_total_bytes(job_id, total_bytes);
                        self.status_message = format!("Direct job #{job_id} connected");
                    }
                    DirectDownloadEvent::Progress {
                        downloaded_bytes,
                        total_bytes,
                        bytes_per_second,
                    } => {
                        self.queue_manager.apply_download_progress(
                            job_id,
                            downloaded_bytes,
                            total_bytes,
                            bytes_per_second,
                        );
                        self.status_message = format!("Downloading job #{job_id}");
                    }
                    DirectDownloadEvent::Retrying {
                        attempt,
                        max_attempts,
                        wait_ms,
                        message,
                    } => {
                        self.status_message = format!(
                            "Retrying direct job #{job_id} ({attempt}/{max_attempts}) in {}s: {message}",
                            wait_ms / 1000
                        );
                    }
                    DirectDownloadEvent::Completed {
                        final_file_path,
                        total_bytes,
                    } => {
                        self.queue_manager.apply_download_progress(
                            job_id,
                            total_bytes,
                            Some(total_bytes),
                            0.0,
                        );
                        self.queue_manager.mark_completed(job_id);
                        scrub_completed_metadata = true;
                        self.status_message =
                            format!("Completed job #{job_id} -> {final_file_path}");
                        finished = true;
                    }
                    DirectDownloadEvent::Failed { message } => {
                        self.queue_manager.fail(job_id, &message);
                        self.status_message = format!("Direct job #{job_id} failed: {message}");
                        finished = true;
                    }
                    DirectDownloadEvent::Paused {
                        downloaded_bytes,
                        total_bytes,
                    } => {
                        self.queue_manager.apply_download_progress(
                            job_id,
                            downloaded_bytes,
                            total_bytes,
                            0.0,
                        );
                        self.queue_manager.pause(job_id);
                        self.status_message = format!("Paused direct job #{job_id}");
                        finished = true;
                    }
                }
            }
        }

        if scrub_completed_metadata {
            self.scrub_completed_direct_metadata(job_id);
        }

        if finished {
            self.active_direct_job_id = None;
            self.active_direct_download = None;
            let _ = save_snapshot(&self.storage_path, self.queue_manager.snapshot());
        }
    }

    fn request_pause_for(&mut self, id: u64) {
        if self.active_direct_job_id == Some(id) {
            if let Some(active_download) = &self.active_direct_download {
                if active_download
                    .commands
                    .send(DirectDownloadCommand::Pause)
                    .is_ok()
                {
                    self.status_message = format!("Pause requested for job #{id}");
                    return;
                }
            }
        }

        #[cfg(feature = "torrent-rqbit")]
        if self.active_torrent_job_id == Some(id)
            && self.active_torrent_session.is_none()
            && self.request_pause_rqbit_torrent(id)
        {
            return;
        }

        self.queue_manager.pause(id);
        self.status_message = format!("Paused job #{id}");
    }

    fn request_resume_for(&mut self, id: u64) {
        #[cfg(feature = "torrent-rqbit")]
        if self.active_torrent_job_id == Some(id)
            && self.active_torrent_session.is_none()
            && self.request_resume_rqbit_torrent(id)
        {
            return;
        }

        let kind = self
            .queue_manager
            .get_record(id)
            .map(|record| record.request.kind.clone());

        match kind {
            Some(DownloadKind::Direct) => self.start_direct_download_for(id),
            Some(DownloadKind::Torrent) => self.start_torrent_download_for(id),
            None => {
                self.status_message = format!("Job #{id} no longer exists");
            }
        }
    }

    fn request_remove_for(&mut self, id: u64) {
        self.pending_delete_confirmation_job_id = None;

        if self.active_direct_job_id == Some(id) {
            self.status_message =
                format!("Direct job #{id} is active. Pause or let it finish before removing it.");
            return;
        }

        #[cfg(feature = "torrent-rqbit")]
        if self.active_torrent_job_id == Some(id)
            && self.active_torrent_session.is_none()
            && self.request_remove_rqbit_torrent(id)
        {
            return;
        }

        if self.active_torrent_job_id == Some(id) && self.active_torrent_session.is_some() {
            self.active_torrent_job_id = None;
            self.active_torrent_plan = None;
            self.active_torrent_session = None;
        }

        if self.queue_manager.remove(id) {
            self.status_message = format!("Removed job #{id} from the queue");
            let _ = save_snapshot(&self.storage_path, self.queue_manager.snapshot());
            self.save_desktop_state();
        } else {
            self.status_message = format!("Job #{id} no longer exists");
        }
    }

    fn request_delete_files_for(&mut self, id: u64) {
        if self.pending_delete_confirmation_job_id != Some(id) {
            self.pending_delete_confirmation_job_id = Some(id);
            self.status_message = format!(
                "Click Confirm Delete for job #{id} to permanently remove its torrent files"
            );
            return;
        }

        self.pending_delete_confirmation_job_id = None;

        if self.active_direct_job_id == Some(id) {
            self.status_message = format!(
                "Direct job #{id} does not support Delete Files yet. Stop the worker first."
            );
            return;
        }

        #[cfg(feature = "torrent-rqbit")]
        if self.active_torrent_job_id == Some(id)
            && self.active_torrent_session.is_none()
            && self.request_delete_rqbit_torrent_files(id)
        {
            return;
        }

        self.status_message = format!(
            "Delete Files is currently available only for the active rqbit torrent session"
        );
    }

    fn poll_browser_bridge(&mut self) {
        let mut captures = Vec::new();
        if let Some(bridge) = &self.browser_bridge {
            while let Ok(payload) = bridge.captures.try_recv() {
                captures.push(payload);
            }
        }

        for payload in captures {
            if self.pending_browser_capture.is_none() {
                self.pending_browser_capture = Some(payload);
                self.status_message = "Browser download is waiting for confirmation".to_owned();
            } else {
                self.status_message =
                    "Another browser download arrived while a confirmation was already open"
                        .to_owned();
            }
        }
    }

    fn start_next_torrent_download(&mut self) {
        let Some(id) = self.queue_manager.start_next_queued_torrent() else {
            self.status_message = "No queued torrent found".to_owned();
            return;
        };

        self.start_torrent_download_for(id);
    }

    fn start_torrent_download_for(&mut self, id: u64) {
        let snapshot = self.queue_manager.snapshot().clone();
        let Some(record) = self.queue_manager.get_record(id).cloned() else {
            self.status_message = "Selected torrent job was not found".to_owned();
            return;
        };

        #[cfg(feature = "torrent-rqbit")]
        if self.active_torrent_job_id.is_none() && record.request.source.starts_with("magnet:") {
            self.queue_manager.resume(id);
            self.rqbit_torrent = Some(
                self.rqbit_engine
                    .spawn_magnet_download(record.request.source.clone()),
            );
            self.active_torrent_job_id = Some(id);
            self.active_torrent_plan = None;
            self.active_torrent_session = None;
            self.rqbit_torrent_name = None;
            self.rqbit_info_hash = None;
            self.rqbit_output_folder = None;
            self.rqbit_file_count = None;
            self.rqbit_files.clear();
            self.rqbit_peer_count = 0;
            self.status_message = format!("Started rqbit torrent job #{id}");
            self.save_desktop_state();
            return;
        }

        let plan = build_torrent_task_plan(
            &record.request,
            &snapshot.downloads_root,
            &snapshot.categories,
        );
        let session = create_torrent_session_snapshot(&plan);
        self.queue_manager.resume(id);
        self.active_torrent_job_id = Some(id);
        self.active_torrent_plan = Some(plan);
        self.active_torrent_session = Some(session);
        self.status_message = format!("Started torrent job #{id}");
        self.save_desktop_state();
    }

    fn tick_torrent_session(&mut self) {
        let Some(job_id) = self.active_torrent_job_id else {
            return;
        };

        let Some(session) = &mut self.active_torrent_session else {
            return;
        };

        let progress = simulate_torrent_progress(session, 12, 18);
        let total_bytes: u64 = session.files.iter().map(|file| file.size_bytes).sum();
        self.queue_manager.apply_torrent_progress(
            job_id,
            progress.progress_percent,
            session.downloaded_bytes,
            total_bytes,
            progress.download_rate_mbps,
            progress.connected_peers,
            &progress.eta_text,
        );

        if progress.progress_percent >= 100.0 {
            self.queue_manager.mark_completed(job_id);
            self.scrub_completed_torrent_metadata(job_id);
            self.status_message = format!("Torrent job #{job_id} completed");
            let _ = save_snapshot(&self.storage_path, self.queue_manager.snapshot());
            self.active_torrent_job_id = None;
            self.active_torrent_plan = None;
            self.active_torrent_session = None;
            self.save_desktop_state();
        }
    }

    #[cfg(feature = "torrent-rqbit")]
    fn poll_rqbit_torrent_events(&mut self) {
        let Some(job_id) = self.active_torrent_job_id else {
            return;
        };

        let mut finished = false;
        let mut scrub_completed_metadata = false;
        if let Some(active_torrent) = &self.rqbit_torrent {
            while let Ok(event) = active_torrent.events.try_recv() {
                match event {
                    RqbitTorrentEvent::SessionStarted => {
                        self.queue_manager.apply_torrent_progress(
                            job_id,
                            0.0,
                            0,
                            1,
                            0.0,
                            0,
                            "Session started",
                        );
                        self.status_message = format!("Torrent job #{job_id} session started");
                    }
                    RqbitTorrentEvent::Paused => {
                        self.queue_manager.pause(job_id);
                        self.status_message = format!("Paused torrent job #{job_id} via rqbit");
                        self.save_desktop_state();
                    }
                    RqbitTorrentEvent::Resumed => {
                        self.queue_manager.resume(job_id);
                        self.status_message = format!("Resumed torrent job #{job_id} via rqbit");
                        self.save_desktop_state();
                    }
                    RqbitTorrentEvent::MetadataResolved {
                        display_name,
                        info_hash,
                        output_folder,
                        file_count,
                        files,
                    } => {
                        self.rqbit_torrent_name = Some(display_name.clone());
                        self.rqbit_info_hash = Some(info_hash);
                        self.rqbit_output_folder = Some(output_folder);
                        self.rqbit_file_count = Some(file_count);
                        self.rqbit_files = files;
                        self.status_message =
                            format!("Torrent job #{job_id} resolved metadata for {display_name}");
                        self.save_desktop_state();
                    }
                    RqbitTorrentEvent::Progress {
                        progress_percent,
                        downloaded_bytes,
                        total_bytes,
                        download_rate_mbps,
                        upload_rate_mbps: _upload_rate_mbps,
                        peers,
                        eta_text,
                    } => {
                        self.rqbit_peer_count = peers;
                        self.queue_manager.apply_torrent_progress(
                            job_id,
                            progress_percent,
                            downloaded_bytes,
                            total_bytes,
                            download_rate_mbps,
                            peers,
                            &eta_text,
                        );
                        self.status_message =
                            format!("Torrent job #{job_id} downloading via rqbit");
                        self.save_desktop_state();
                    }
                    RqbitTorrentEvent::Completed => {
                        self.queue_manager.mark_completed(job_id);
                        scrub_completed_metadata = true;
                        self.status_message = format!("Torrent job #{job_id} completed via rqbit");
                        finished = true;
                    }
                    RqbitTorrentEvent::Removed => {
                        self.pending_delete_confirmation_job_id = None;
                        self.queue_manager.remove(job_id);
                        self.status_message = format!("Removed torrent job #{job_id} via rqbit");
                        finished = true;
                    }
                    RqbitTorrentEvent::Deleted => {
                        self.pending_delete_confirmation_job_id = None;
                        self.queue_manager.remove(job_id);
                        self.status_message =
                            format!("Deleted torrent job #{job_id} and its files via rqbit");
                        finished = true;
                    }
                    RqbitTorrentEvent::Failed(message) => {
                        self.pending_delete_confirmation_job_id = None;
                        self.queue_manager.fail(job_id, &message);
                        self.status_message =
                            format!("Torrent job #{job_id} failed via rqbit: {message}");
                        finished = true;
                    }
                }
            }
        }

        if scrub_completed_metadata {
            self.scrub_completed_torrent_metadata(job_id);
        }

        if finished {
            self.active_torrent_job_id = None;
            self.active_torrent_plan = None;
            self.active_torrent_session = None;
            self.clear_rqbit_session_metadata();
            let _ = save_snapshot(&self.storage_path, self.queue_manager.snapshot());
            self.save_desktop_state();
        }
    }

    #[cfg(not(feature = "torrent-rqbit"))]
    fn torrent_engine_label(&self) -> &'static str {
        "Simulated"
    }

    #[cfg(feature = "torrent-rqbit")]
    fn torrent_engine_label(&self) -> &'static str {
        "librqbit"
    }

    #[cfg(feature = "torrent-rqbit")]
    fn restore_rqbit_job_if_needed(&mut self) {
        if self.privacy_settings.minimize_torrent_metadata_retention() {
            self.pending_rqbit_restore = None;
            return;
        }

        if self.active_torrent_job_id.is_some() || self.rqbit_torrent.is_some() {
            return;
        }

        let Some(saved_state) = self.pending_rqbit_restore.take() else {
            return;
        };

        let can_restore = self
            .queue_manager
            .get_record(saved_state.queue_job_id)
            .map(|record| saved_state.matches_torrent_job(&record.request))
            .unwrap_or(false);

        if can_restore {
            self.status_message = format!("Restoring torrent job #{}", saved_state.queue_job_id);
            self.start_torrent_download_for(saved_state.queue_job_id);
        } else {
            self.clear_rqbit_session_metadata();
            self.save_desktop_state();
        }
    }

    #[cfg(feature = "torrent-rqbit")]
    fn request_pause_rqbit_torrent(&mut self, id: u64) -> bool {
        if let Some(active_torrent) = &self.rqbit_torrent
            && active_torrent
                .commands
                .send(RqbitTorrentCommand::Pause)
                .is_ok()
        {
            self.status_message = format!("Pause requested for torrent job #{id}");
            return true;
        }

        false
    }

    #[cfg(feature = "torrent-rqbit")]
    fn request_resume_rqbit_torrent(&mut self, id: u64) -> bool {
        if let Some(active_torrent) = &self.rqbit_torrent
            && active_torrent
                .commands
                .send(RqbitTorrentCommand::Resume)
                .is_ok()
        {
            self.status_message = format!("Resume requested for torrent job #{id}");
            return true;
        }

        false
    }

    #[cfg(feature = "torrent-rqbit")]
    fn request_remove_rqbit_torrent(&mut self, id: u64) -> bool {
        if let Some(active_torrent) = &self.rqbit_torrent
            && active_torrent
                .commands
                .send(RqbitTorrentCommand::Remove)
                .is_ok()
        {
            self.status_message = format!("Remove requested for torrent job #{id}");
            return true;
        }

        false
    }

    #[cfg(feature = "torrent-rqbit")]
    fn request_delete_rqbit_torrent_files(&mut self, id: u64) -> bool {
        if let Some(active_torrent) = &self.rqbit_torrent
            && active_torrent
                .commands
                .send(RqbitTorrentCommand::DeleteFiles)
                .is_ok()
        {
            self.status_message = format!("Delete Files requested for torrent job #{id}");
            return true;
        }

        false
    }

    fn save_desktop_state(&self) {
        let state = DesktopPersistedState {
            expanded_details_job_id: self.expanded_details_job_id,
            privacy: self.privacy_settings.clone(),
            #[cfg(feature = "torrent-rqbit")]
            rqbit: self.active_torrent_job_id.and_then(|queue_job_id| {
                if self.privacy_settings.minimize_torrent_metadata_retention() {
                    return None;
                }
                let record = self.queue_manager.get_record(queue_job_id)?;
                if record.request.kind != DownloadKind::Torrent
                    || !record.request.source.starts_with("magnet:")
                {
                    return None;
                }

                Some(RqbitPersistedState {
                    queue_job_id,
                    magnet_uri: record.request.source.clone(),
                    torrent_name: self.rqbit_torrent_name.clone(),
                    info_hash: self.rqbit_info_hash.clone(),
                    output_folder: self.rqbit_output_folder.clone(),
                    file_count: self.rqbit_file_count,
                    files: self.rqbit_files.clone(),
                    peer_count: self.rqbit_peer_count,
                })
            }),
            #[cfg(not(feature = "torrent-rqbit"))]
            rqbit: None,
        };

        let _ = state.save(&self.desktop_state_path);
    }
}

fn auto_register_magnet_protocol() -> Option<String> {
    #[cfg(windows)]
    {
        match register_magnet_protocol_handler() {
            Ok(()) => Some("Magnet links registered automatically".to_owned()),
            Err(err) => Some(format!("Magnet auto-registration failed: {err}")),
        }
    }

    #[cfg(not(windows))]
    {
        None
    }
}

fn resolve_browser_extension_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let current_dir = std::env::current_dir().ok();

    let mut candidates = Vec::new();
    if let Some(dir) = exe_dir {
        candidates.push(dir.join("browser-extension"));
        candidates.push(dir.join("extensions").join("browser"));
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join("browser-extension"));
            candidates.push(parent.join("extensions").join("browser"));
        }
        if let Some(grandparent) = dir.parent().and_then(|parent| parent.parent()) {
            candidates.push(grandparent.join("extensions").join("browser"));
        }
    }

    if let Some(dir) = current_dir {
        candidates.push(dir.join("extensions").join("browser"));
        candidates.push(dir.join("browser-extension"));
    }

    candidates.into_iter().find(|path| path.is_dir())
}

#[cfg(windows)]
fn register_magnet_protocol_handler() -> Result<(), String> {
    let exe_path =
        std::env::current_exe().map_err(|err| format!("current exe lookup failed: {err}"))?;
    let command = format!("\"{}\" \"%1\"", exe_path.display());

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let classes_root = hkcu
        .create_subkey("Software\\Classes\\magnet")
        .map_err(|err| format!("open magnet class failed: {err}"))?
        .0;
    classes_root
        .set_value("", &"URL:Magnet Protocol")
        .map_err(|err| format!("set class description failed: {err}"))?;
    classes_root
        .set_value("URL Protocol", &"")
        .map_err(|err| format!("set URL Protocol failed: {err}"))?;
    classes_root
        .set_value("FriendlyTypeName", &"NebulaDM Magnet Link")
        .map_err(|err| format!("set friendly name failed: {err}"))?;

    let icon_key = classes_root
        .create_subkey("DefaultIcon")
        .map_err(|err| format!("create icon key failed: {err}"))?
        .0;
    icon_key
        .set_value("", &format!("\"{}\",0", exe_path.display()))
        .map_err(|err| format!("set icon failed: {err}"))?;

    let command_key = classes_root
        .create_subkey("shell\\open\\command")
        .map_err(|err| format!("create command key failed: {err}"))?
        .0;
    command_key
        .set_value("", &command)
        .map_err(|err| format!("set open command failed: {err}"))?;

    Ok(())
}
