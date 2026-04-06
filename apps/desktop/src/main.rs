#![cfg_attr(windows, windows_subsystem = "windows")]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use arboard::Clipboard;
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
use egui::{IconData, ViewportClass, ViewportCommand, ViewportId};
use semver::Version;
use serde::Deserialize;
#[cfg(windows)]
use tray_icon::{
    Icon as TrayIconImage, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};
#[cfg(feature = "torrent-rqbit")]
use shared::RqbitPersistedState;
#[cfg(feature = "torrent-rqbit")]
use shared::TorrentFileEntry;
use shared::{
    BrowserCapturePayload, DesktopPersistedState, DownloadKind, DownloadRecord, DownloadRequest,
    DownloadStatus, DuplicateStrategy, PostDownloadAction, PrivacySettings, QueueView,
    TorrentSessionSnapshot,
};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use winreg::{RegKey, enums::HKEY_CURRENT_USER};
#[cfg(windows)]
use winrt_notification::{Duration as ToastDuration, Sound, Toast};

const BACKGROUND: egui::Color32 = egui::Color32::from_rgb(22, 22, 26);
const PANEL: egui::Color32 = egui::Color32::from_rgb(33, 33, 39);
const PANEL_ALT: egui::Color32 = egui::Color32::from_rgb(42, 42, 50);
const PANEL_HIGHLIGHT: egui::Color32 = egui::Color32::from_rgb(31, 212, 228);
const PANEL_SUBTLE: egui::Color32 = egui::Color32::from_rgb(26, 26, 32);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(31, 212, 228);
const ACCENT_WARM: egui::Color32 = egui::Color32::from_rgb(241, 173, 225);
const SUCCESS: egui::Color32 = egui::Color32::from_rgb(120, 237, 220);
const DANGER: egui::Color32 = egui::Color32::from_rgb(237, 108, 88);
const MUTED_TEXT: egui::Color32 = egui::Color32::from_rgb(177, 180, 188);
const BRIGHT_TEXT: egui::Color32 = egui::Color32::from_rgb(244, 246, 250);
const OUTLINE: egui::Color32 = egui::Color32::from_rgb(66, 71, 83);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let launch_request = args.iter().cloned().find_map(parse_launch_request);
    let start_in_background = args.iter().any(|arg| arg == "--background");
    let viewport = match load_app_icon() {
        Some(icon) => egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("NebulaDM")
            .with_visible(!start_in_background)
            .with_icon(icon),
        None => egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("NebulaDM")
            .with_visible(!start_in_background),
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "NebulaDM",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(DesktopApp::new(
                launch_request.clone(),
                start_in_background,
            )))
        }),
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

#[cfg(windows)]
fn load_tray_icon() -> Option<TrayIconImage> {
    let bytes = include_bytes!("../../../assets/nebuladm-logo.png");
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    TrayIconImage::from_rgba(image.into_raw(), width, height).ok()
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

#[derive(Debug, Clone)]
struct AppNotification {
    id: u64,
    title: String,
    body: String,
    created_at: Instant,
}

#[derive(Debug, Clone, Deserialize)]
struct UpdateManifest {
    version: String,
    installer_url: String,
    notes_url: Option<String>,
}

struct DesktopApp {
    selected_tab: DownloadTab,
    queue_manager: QueueManager,
    selected_view: QueueView,
    storage_path: PathBuf,
    desktop_state_path: PathBuf,
    privacy_settings: PrivacySettings,
    run_on_startup: bool,
    clipboard_watch_enabled: bool,
    native_notifications_enabled: bool,
    update_feed_url: String,
    duplicate_strategy: DuplicateStrategy,
    post_download_action: PostDownloadAction,
    start_in_background: bool,
    new_name: String,
    new_source: String,
    batch_import_sources: String,
    new_kind: DownloadKind,
    status_message: String,
    recent_download_targets: Vec<String>,
    notifications: Vec<AppNotification>,
    notification_serial: u64,
    last_clipboard_value: Option<String>,
    last_clipboard_poll_at: Instant,
    show_setup_center: bool,
    pending_delete_history_confirmation: bool,
    pending_delete_confirmation_job_id: Option<u64>,
    expanded_details_job_id: Option<u64>,
    pending_browser_capture: Option<BrowserCapturePayload>,
    pending_browser_capture_save_folder: String,
    pending_launch_start_job_id: Option<u64>,
    #[cfg(windows)]
    tray_icon: Option<TrayIcon>,
    #[cfg(windows)]
    tray_show_id: Option<MenuId>,
    #[cfg(windows)]
    tray_hide_id: Option<MenuId>,
    #[cfg(windows)]
    tray_quit_id: Option<MenuId>,
    #[cfg(windows)]
    tray_pause_id: Option<MenuId>,
    #[cfg(windows)]
    tray_resume_id: Option<MenuId>,
    #[cfg(windows)]
    tray_recent_id: Option<MenuId>,
    #[cfg(windows)]
    quit_requested: bool,
    active_direct_job_id: Option<u64>,
    active_direct_download: Option<ActiveDirectDownload>,
    browser_bridge: Option<BrowserBridge>,
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
    fn card_frame(&self, fill: egui::Color32) -> egui::Frame {
        egui::Frame::new()
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, OUTLINE))
            .corner_radius(20.0)
            .inner_margin(egui::Margin::symmetric(18, 16))
    }

    fn section_heading(&self, ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.label(
            egui::RichText::new(title)
                .size(18.0)
                .strong()
                .color(BRIGHT_TEXT),
        );
        ui.label(egui::RichText::new(subtitle).color(MUTED_TEXT));
        ui.add_space(8.0);
    }

    fn stat_card(
        &self,
        ui: &mut egui::Ui,
        title: &str,
        value: String,
        accent: egui::Color32,
        detail: &str,
    ) {
        self.card_frame(PANEL_SUBTLE).show(ui, |ui| {
            ui.label(egui::RichText::new(title).color(MUTED_TEXT));
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(value)
                    .size(24.0)
                    .strong()
                    .color(accent),
            );
            ui.add_space(4.0);
            ui.label(egui::RichText::new(detail).color(MUTED_TEXT));
        });
    }

    fn compact_action_button(
        &self,
        ui: &mut egui::Ui,
        label: &str,
        fill: egui::Color32,
        enabled: bool,
        tooltip: &str,
        emphasized: bool,
    ) -> egui::Response {
        ui.add_enabled(
            enabled,
            egui::Button::new(
                egui::RichText::new(label)
                    .size(14.0)
                    .color(if emphasized { BRIGHT_TEXT } else { MUTED_TEXT }),
            )
                .fill(if emphasized {
                    fill
                } else {
                    fill.linear_multiply(0.45)
                })
                .corner_radius(10.0)
                .min_size(egui::vec2(34.0, 28.0)),
        )
        .on_hover_text(tooltip)
    }

    fn new(launch_request: Option<DownloadRequest>, start_in_background: bool) -> Self {
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

        let browser_bridge = start_browser_bridge("127.0.0.1:35791").ok();
        let auto_register_status = auto_register_magnet_protocol();
        let startup_status = sync_run_on_startup(desktop_state.run_on_startup);
        let status_message = match auto_register_status {
            Some(message) => format!("{status_message} | {message}"),
            None => status_message,
        };
        let status_message = match startup_status {
            Some(message) => format!("{status_message} | {message}"),
            None => status_message,
        };
        let pending_browser_capture_save_folder = display_path(&download_root);

        let mut app = Self {
            selected_tab,
            queue_manager,
            selected_view,
            storage_path,
            desktop_state_path,
            privacy_settings: desktop_state.privacy.clone(),
            run_on_startup: desktop_state.run_on_startup,
            clipboard_watch_enabled: desktop_state.clipboard_watch_enabled,
            native_notifications_enabled: desktop_state.native_notifications_enabled,
            update_feed_url: desktop_state.update_feed_url.clone(),
            duplicate_strategy: desktop_state.duplicate_strategy,
            post_download_action: desktop_state.post_download_action,
            start_in_background,
            new_name: String::new(),
            new_source: String::new(),
            batch_import_sources: String::new(),
            new_kind: DownloadKind::Direct,
            status_message,
            recent_download_targets: Vec::new(),
            notifications: Vec::new(),
            notification_serial: 0,
            last_clipboard_value: None,
            last_clipboard_poll_at: Instant::now(),
            show_setup_center: false,
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
            pending_browser_capture_save_folder,
            pending_launch_start_job_id,
            #[cfg(windows)]
            tray_icon: None,
            #[cfg(windows)]
            tray_show_id: None,
            #[cfg(windows)]
            tray_hide_id: None,
            #[cfg(windows)]
            tray_quit_id: None,
            #[cfg(windows)]
            tray_pause_id: None,
            #[cfg(windows)]
            tray_resume_id: None,
            #[cfg(windows)]
            tray_recent_id: None,
            #[cfg(windows)]
            quit_requested: false,
            active_direct_job_id: None,
            active_direct_download: None,
            browser_bridge,
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
        };

        #[cfg(windows)]
        app.init_tray_icon();

        app
    }
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_theme(ctx);
        #[cfg(windows)]
        self.handle_tray_events(ctx);
        #[cfg(windows)]
        self.handle_root_close_to_tray(ctx);
        self.handle_launch_request_if_needed();
        #[cfg(feature = "torrent-rqbit")]
        self.restore_rqbit_job_if_needed();
        self.poll_clipboard_for_download_links();
        self.poll_direct_events();
        self.poll_browser_bridge();
        #[cfg(feature = "torrent-rqbit")]
        self.poll_rqbit_torrent_events();
        self.tick_torrent_session();
        let has_active_work = self.active_direct_job_id.is_some()
            || self.active_torrent_job_id.is_some()
            || self.pending_browser_capture.is_some()
            || !self.notifications.is_empty();
        ctx.request_repaint_after(std::time::Duration::from_millis(if has_active_work {
            150
        } else {
            1000
        }));
        let snapshot = self.queue_manager.snapshot().clone();

        #[cfg(windows)]
        if self.quit_requested {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
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
                    .fill(BACKGROUND)
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL_SUBTLE)
                    .stroke(egui::Stroke::new(0.0, egui::Color32::TRANSPARENT))
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("NebulaDM")
                                    .size(22.0)
                                    .strong()
                                    .color(BRIGHT_TEXT),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "A focused download command center for direct links, browser handoff, and torrents.",
                                )
                                .color(MUTED_TEXT),
                            );
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            self.pill(
                                ui,
                                if self.privacy_settings.privacy_mode {
                                    "Privacy On"
                                } else {
                                    "Privacy Relaxed"
                                },
                                if self.privacy_settings.privacy_mode {
                                    SUCCESS
                                } else {
                                    ACCENT_WARM
                                },
                            );
                            self.pill(
                                ui,
                                if self.run_on_startup {
                                    "Background Ready"
                                } else {
                                    "Manual Launch"
                                },
                                ACCENT,
                            );
                        });
                    });
                });
            });

        egui::SidePanel::left("sidebar")
            .min_width(240.0)
            .max_width(240.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL_SUBTLE)
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(
                            egui::RichText::new("Nebula DM")
                                .strong()
                                .color(BRIGHT_TEXT),
                        );
                    });
                    ui.add_space(14.0);
                    egui::Frame::new()
                        .fill(PANEL_SUBTLE)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
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
                            .fill(if selected { PANEL_HIGHLIGHT } else { PANEL_SUBTLE })
                            .corner_radius(14.0)
                            .min_size(egui::vec2(ui.available_width(), 40.0));

                            if ui.add(button).clicked() {
                                self.selected_view = view;
                            }
                            ui.add_space(6.0);
                        }
                    });

                    ui.add_space(10.0);
                    self.card_frame(PANEL).show(ui, |ui| {
                        self.section_heading(ui, "Workspace", "A fast snapshot of the app and current engine.");
                        ui.label(
                            egui::RichText::new(format!("Active transfers: {}", active_count(&snapshot)))
                                .color(ACCENT)
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new(format!(
                                "Downloaded so far: {:.1} MB",
                                total_downloaded_mb(&snapshot)
                            ))
                            .color(SUCCESS),
                        );
                        ui.label(
                            egui::RichText::new(format!("Torrent engine: {}", self.torrent_engine_label()))
                                .color(ACCENT_WARM),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(if self.start_in_background {
                                "Launches quietly and stays ready in the tray."
                            } else {
                                "Opens normally and stays visible until you hide it."
                            })
                            .color(MUTED_TEXT),
                        );
                        #[cfg(feature = "torrent-rqbit")]
                        if let Some(name) = &self.rqbit_torrent_name {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new("Live Torrent").strong().color(BRIGHT_TEXT));
                            ui.label(egui::RichText::new(name).color(BRIGHT_TEXT));
                            ui.label(
                                egui::RichText::new(format!("Peers connected: {}", self.rqbit_peer_count))
                                    .color(MUTED_TEXT),
                            );
                        }
                    });

                    ui.add_space(10.0);
                    self.card_frame(PANEL).show(ui, |ui| {
                        self.section_heading(ui, "Preferences", "Privacy, startup behavior, duplicates, and completion flow.");
                        let mut privacy_mode = self.privacy_settings.privacy_mode;
                        if ui.checkbox(&mut privacy_mode, "Privacy Mode").changed() {
                            self.set_privacy_mode(privacy_mode);
                        }
                        ui.add_space(8.0);
                        let mut run_on_startup = self.run_on_startup;
                        if ui.checkbox(&mut run_on_startup, "Run on startup in background").changed() {
                            self.set_run_on_startup(run_on_startup);
                        }
                        if ui
                            .checkbox(&mut self.clipboard_watch_enabled, "Watch clipboard for links")
                            .changed()
                        {
                            self.save_desktop_state();
                        }
                        if ui
                            .checkbox(
                                &mut self.native_notifications_enabled,
                                "Use native Windows notifications",
                            )
                            .changed()
                        {
                            self.save_desktop_state();
                        }
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Duplicate File Rule").color(MUTED_TEXT));
                        let previous_duplicate_strategy = self.duplicate_strategy;
                        egui::ComboBox::from_id_salt("duplicate_strategy")
                            .selected_text(self.duplicate_strategy.to_string())
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.duplicate_strategy,
                                    DuplicateStrategy::Rename,
                                    "Rename",
                                );
                                ui.selectable_value(
                                    &mut self.duplicate_strategy,
                                    DuplicateStrategy::Overwrite,
                                    "Overwrite",
                                );
                                ui.selectable_value(
                                    &mut self.duplicate_strategy,
                                    DuplicateStrategy::Skip,
                                    "Skip",
                                );
                            });
                        if self.duplicate_strategy != previous_duplicate_strategy {
                            self.save_desktop_state();
                        }
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("After Download").color(MUTED_TEXT));
                        let previous_post_download_action = self.post_download_action;
                        egui::ComboBox::from_id_salt("post_download_action")
                            .selected_text(self.post_download_action.to_string())
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.post_download_action,
                                    PostDownloadAction::None,
                                    "Do nothing",
                                );
                                ui.selectable_value(
                                    &mut self.post_download_action,
                                    PostDownloadAction::OpenFile,
                                    "Open file",
                                );
                                ui.selectable_value(
                                    &mut self.post_download_action,
                                    PostDownloadAction::OpenFolder,
                                    "Open folder",
                                );
                            });
                        if self.post_download_action != previous_post_download_action {
                            self.save_desktop_state();
                        }
                    });

                    ui.add_space(10.0);
                    self.card_frame(PANEL).show(ui, |ui| {
                        self.section_heading(ui, "Tools", "Quick actions for setup, associations, and cleanup.");
                        if ui
                            .add(
                                egui::Button::new("Open Setup Center")
                                    .fill(PANEL_HIGHLIGHT)
                                    .corner_radius(14.0)
                                    .min_size(egui::vec2(ui.available_width(), 38.0)),
                            )
                            .clicked()
                        {
                            self.show_setup_center = true;
                        }
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
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(14, 12)),
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
                ui.add_space(12.0);
                self.card_frame(PANEL).show(ui, |ui| {
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
                    ui.add_space(12.0);
                    ui.columns(3, |columns| {
                        self.stat_card(
                            &mut columns[0],
                            "Visible Items",
                            visible_records.len().to_string(),
                            ACCENT,
                            "Shown in this tab and filter",
                        );
                        self.stat_card(
                            &mut columns[1],
                            "Clipboard Watcher",
                            if self.clipboard_watch_enabled {
                                "On".to_owned()
                            } else {
                                "Off".to_owned()
                            },
                            if self.clipboard_watch_enabled {
                                SUCCESS
                            } else {
                                MUTED_TEXT
                            },
                            "Auto-detect copied links",
                        );
                        self.stat_card(
                            &mut columns[2],
                            "Tray Mode",
                            if self.run_on_startup {
                                "Ready".to_owned()
                            } else {
                                "Manual".to_owned()
                            },
                            ACCENT_WARM,
                            "Background-friendly startup",
                        );
                    });
                });
                ui.add_space(10.0);
                self.render_quick_add_panel(ui, &snapshot);

                ui.add_space(12.0);

                if visible_records.is_empty() {
                    self.card_frame(PANEL).show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("Nothing here yet")
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
                    self.card_frame(PANEL).show(ui, |ui| {
                        ui.columns(5, |columns| {
                            columns[0].label(egui::RichText::new("File Name").strong().color(MUTED_TEXT));
                            columns[1].label(egui::RichText::new("Status").strong().color(MUTED_TEXT));
                            columns[2].label(egui::RichText::new("Speed").strong().color(MUTED_TEXT));
                            columns[3].label(egui::RichText::new("Progress").strong().color(MUTED_TEXT));
                            columns[4].label(egui::RichText::new("Actions").strong().color(MUTED_TEXT));
                        });
                    });
                    ui.add_space(8.0);
                    for record in visible_records {
                    let plan = plan_download(
                        &record.request,
                        &snapshot.downloads_root,
                        &snapshot.categories,
                    );

                    let row_response = egui::Frame::new()
                        .fill(PANEL)
                        .stroke(egui::Stroke::new(1.0, OUTLINE.linear_multiply(0.7)))
                        .corner_radius(16.0)
                        .inner_margin(egui::Margin::symmetric(16, 14))
                        .show(ui, |ui| {
                        let row_hovered = ui.rect_contains_pointer(ui.max_rect());
                        let accent_fill = if row_hovered {
                            match record.request.kind {
                                DownloadKind::Direct => ACCENT.linear_multiply(0.05),
                                DownloadKind::Torrent => ACCENT_WARM.linear_multiply(0.05),
                            }
                        } else {
                            PANEL
                        };
                        ui.painter().rect_filled(ui.max_rect(), 16.0, accent_fill);
                        ui.painter().rect_stroke(
                            ui.max_rect(),
                            16.0,
                            egui::Stroke::new(
                                if row_hovered { 1.2 } else { 1.0 },
                                if row_hovered {
                                    match record.request.kind {
                                        DownloadKind::Direct => ACCENT.linear_multiply(0.7),
                                        DownloadKind::Torrent => ACCENT_WARM.linear_multiply(0.7),
                                    }
                                } else {
                                    OUTLINE.linear_multiply(0.7)
                                },
                            ),
                            egui::StrokeKind::Inside,
                        );
                        ui.columns(5, |columns| {
                            columns[0].vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(&record.request.file_name)
                                        .strong()
                                        .color(match record.request.kind {
                                            DownloadKind::Direct => ACCENT,
                                            DownloadKind::Torrent => ACCENT_WARM,
                                        }),
                                );
                                ui.label(egui::RichText::new(&record.request.source).small().color(MUTED_TEXT));
                            });
                            columns[1].vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(record.request.kind.to_string())
                                        .strong()
                                        .color(match record.request.kind {
                                            DownloadKind::Direct => ACCENT,
                                            DownloadKind::Torrent => ACCENT_WARM,
                                        }),
                                );
                                ui.label(
                                    egui::RichText::new(record.status.to_string())
                                        .color(self.status_color(&record.status.to_string())),
                                );
                            });
                            columns[2].vertical(|ui| {
                                ui.monospace(format!("{:.2} MB/s", record.speed_mbps));
                                ui.label(egui::RichText::new(record.eta_text.clone()).color(MUTED_TEXT));
                            });
                            columns[3].vertical(|ui| {
                                ui.add(
                                    egui::ProgressBar::new(record.progress_percent / 100.0)
                                        .desired_width(110.0)
                                        .fill(match record.request.kind {
                                            DownloadKind::Direct => ACCENT,
                                            DownloadKind::Torrent => ACCENT_WARM,
                                        })
                                        .text(format!("{:.0}%", record.progress_percent)),
                                );
                                ui.monospace(format!(
                                    "{:.1}/{:.1} MB",
                                    record.downloaded_mb, record.total_mb
                                ));
                            });
                            columns[4].horizontal(|ui| {
                            let can_pause = matches!(
                                record.status,
                                DownloadStatus::Downloading
                            );
                            let can_resume = matches!(
                                record.status,
                                DownloadStatus::Paused | DownloadStatus::Queued | DownloadStatus::Failed
                            );
                            if self
                                .compact_action_button(
                                    ui,
                                    "⏸",
                                    PANEL_ALT,
                                    can_pause,
                                    "Pause download",
                                    row_hovered,
                                )
                                .clicked()
                            {
                                self.pending_delete_confirmation_job_id = None;
                                self.request_pause_for(record.id);
                            }
                            if self
                                .compact_action_button(
                                    ui,
                                    "▶",
                                    PANEL_HIGHLIGHT,
                                    can_resume,
                                    "Resume download",
                                    row_hovered,
                                )
                                .clicked()
                            {
                                self.pending_delete_confirmation_job_id = None;
                                self.request_resume_for(record.id);
                            }
                            let details_label = if self.expanded_details_job_id == Some(record.id) {
                                "▾"
                            } else {
                                "⋯"
                            };
                            if self
                                .compact_action_button(
                                    ui,
                                    details_label,
                                    PANEL_ALT,
                                    true,
                                    "Show details",
                                    row_hovered || self.expanded_details_job_id == Some(record.id),
                                )
                                .clicked()
                            {
                                self.toggle_details_for(record.id);
                            }
                            if self
                                .compact_action_button(
                                    ui,
                                    "✕",
                                    DANGER,
                                    true,
                                    "Remove from queue",
                                    row_hovered,
                                )
                                .clicked()
                            {
                                self.pending_delete_confirmation_job_id = None;
                                self.request_remove_for(record.id);
                            }
                            });
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

        self.render_browser_capture_confirmation_popup(ctx);
        self.render_setup_center(ctx);
        self.render_notification_toasts(ctx);
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
        self.card_frame(PANEL)
            .show(ui, |ui| {
                self.section_heading(
                    ui,
                    "Quick Add",
                    "Enter a URL or magnet link, then launch it directly or route it through torrents.",
                );
                ui.add_space(10.0);
                ui.columns(2, |columns| {
                    columns[0].label(egui::RichText::new("Source URL or Magnet").color(MUTED_TEXT));
                    columns[1].label(egui::RichText::new("File Name (optional)").color(MUTED_TEXT));
                    columns[0].add(
                        egui::TextEdit::singleline(&mut self.new_source)
                            .hint_text("Enter URL or Magnet")
                            .desired_width(f32::INFINITY),
                    );
                    columns[1].add(
                        egui::TextEdit::singleline(&mut self.new_name)
                            .hint_text("File Name (optional)")
                            .desired_width(f32::INFINITY),
                    );
                });
                if !self.new_source.trim().is_empty() {
                    self.new_kind = infer_download_kind(&self.new_source);
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let button_width = ((ui.available_width() - 12.0) / 2.0).max(160.0);
                    if ui
                        .add(
                            egui::Button::new("Direct Download")
                                .fill(ACCENT)
                                .corner_radius(12.0)
                                .min_size(egui::vec2(button_width, 38.0)),
                        )
                        .clicked()
                    {
                        self.new_kind = DownloadKind::Direct;
                        self.add_manual_download();
                        self.start_next_direct_download();
                    }
                    if ui
                        .add(
                            egui::Button::new("Torrent Download")
                                .fill(ACCENT_WARM)
                                .corner_radius(12.0)
                                .min_size(egui::vec2(button_width, 38.0)),
                        )
                        .clicked()
                    {
                        self.new_kind = DownloadKind::Torrent;
                        self.add_manual_download();
                        self.start_next_torrent_download();
                    }
                });
                ui.add_space(10.0);
                self.card_frame(PANEL_SUBTLE).show(ui, |ui| {
                    ui.label(egui::RichText::new(format!("Status: {}", self.status_message)).color(ACCENT).strong());
                    ui.label(egui::RichText::new(self.quick_add_hint()).color(ACCENT_WARM).strong());
                });
                ui.add_space(12.0);
                ui.label(egui::RichText::new("Batch Import").color(MUTED_TEXT));
                ui.add(
                    egui::TextEdit::multiline(&mut self.batch_import_sources)
                        .desired_rows(3)
                        .desired_width(ui.available_width().max(520.0))
                        .hint_text("Paste one URL or magnet per line"),
                );
                if ui
                    .add(
                        egui::Button::new("Import Batch URLs")
                            .fill(PANEL_HIGHLIGHT)
                            .corner_radius(12.0),
                    )
                    .clicked()
                {
                    self.import_batch_downloads();
                }
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
    fn render_browser_capture_confirmation_contents(&mut self, ui: &mut egui::Ui) {
        let Some(payload) = self.pending_browser_capture.clone() else {
            return;
        };

        let preview_request = payload
            .clone()
            .into_request()
            .with_custom_target_folder(Some(self.pending_browser_capture_save_folder.clone()));
        let preview_plan = plan_download(
            &preview_request,
            &self.queue_manager.snapshot().downloads_root,
            &self.queue_manager.snapshot().categories,
        );

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
        ui.label(egui::RichText::new(format!("Source: {}", payload.source)).color(MUTED_TEXT));
        if let Some(referrer) = &payload.referrer
            && !self.privacy_settings.minimize_browser_metadata_retention()
        {
            ui.label(egui::RichText::new(format!("Referrer: {referrer}")).color(MUTED_TEXT));
        }
        ui.add_space(10.0);
        ui.label(egui::RichText::new("Save To").color(MUTED_TEXT));
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.pending_browser_capture_save_folder)
                    .desired_width(320.0),
            );
            if ui.button("Browse").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.pending_browser_capture_save_folder)
                    .pick_folder()
                {
                    self.pending_browser_capture_save_folder = display_path(&path);
                }
            }
        });
        ui.label(
            egui::RichText::new(format!("Planned Target Folder: {}", preview_plan.target_folder))
                .color(ACCENT_WARM),
        );
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
    }

    fn render_browser_capture_confirmation_popup(&mut self, ctx: &egui::Context) {
        let Some(payload) = self.pending_browser_capture.clone() else {
            return;
        };

        let viewport_id = ViewportId::from_hash_of("browser-capture-confirmation");
        let title = format!("Confirm Download: {}", payload.file_name);
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([520.0, 320.0])
            .with_min_inner_size([480.0, 280.0])
            .with_always_on_top();
        let builder = if let Some(icon) = load_app_icon() {
            builder.with_icon(icon)
        } else {
            builder
        };

        ctx.show_viewport_immediate(viewport_id, builder, |ctx, class| {
            let render_ui = |ui: &mut egui::Ui, app: &mut Self| {
                app.render_browser_capture_confirmation_contents(ui);
            };

            match class {
                ViewportClass::Embedded => {
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
                        .show(ctx, |ui| render_ui(ui, self));
                }
                ViewportClass::Immediate => {
                    egui::CentralPanel::default()
                        .frame(
                            egui::Frame::new()
                                .fill(BACKGROUND)
                                .inner_margin(egui::Margin::symmetric(18, 18)),
                        )
                        .show(ctx, |ui| render_ui(ui, self));
                }
                _ => {}
            }
        });
    }

    fn accept_pending_browser_capture(&mut self) {
        let Some(payload) = self.pending_browser_capture.take() else {
            return;
        };

        let mut request = payload.into_request();
        let custom_target = self.pending_browser_capture_save_folder.trim().to_owned();
        if !custom_target.is_empty() {
            request = request.with_custom_target_folder(Some(custom_target));
        }
        if self.privacy_settings.minimize_browser_metadata_retention() {
            request.clear_browser_context();
        }
        let id = self.queue_manager.add_download_request(request, true);
        self.selected_view = QueueView::BrowserCapture;
        self.pending_browser_capture_save_folder = display_path(&resolve_download_root());
        self.status_message = format!("Accepted browser download into job #{id}");
        let _ = save_snapshot(&self.storage_path, self.queue_manager.snapshot());
        self.save_desktop_state();
    }

    fn reject_pending_browser_capture(&mut self) {
        if let Some(payload) = self.pending_browser_capture.take() {
            self.pending_browser_capture_save_folder = display_path(&resolve_download_root());
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

    fn set_run_on_startup(&mut self, enabled: bool) {
        self.run_on_startup = enabled;
        self.status_message = match sync_run_on_startup(enabled) {
            Some(message) => message,
            None => {
                if enabled {
                    "Run on startup enabled".to_owned()
                } else {
                    "Run on startup disabled".to_owned()
                }
            }
        };
        self.save_desktop_state();
    }

    #[cfg(windows)]
    fn init_tray_icon(&mut self) {
        if self.tray_icon.is_some() {
            return;
        }

        let Some(icon) = load_tray_icon() else {
            self.status_message = "Tray icon could not load the app logo".to_owned();
            return;
        };

        let menu = Menu::new();
        let show_item = MenuItem::new("Open NebulaDM", true, None);
        let hide_item = MenuItem::new("Hide To Tray", true, None);
        let pause_item = MenuItem::new("Pause Active", true, None);
        let resume_item = MenuItem::new("Resume Active", true, None);
        let recent_item = MenuItem::new("Open Recent Download Folder", true, None);
        let quit_item = MenuItem::new("Quit", true, None);
        let _ = menu.append_items(&[
            &show_item,
            &hide_item,
            &pause_item,
            &resume_item,
            &recent_item,
            &PredefinedMenuItem::separator(),
            &quit_item,
        ]);

        match TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("NebulaDM")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
        {
            Ok(tray_icon) => {
                self.tray_show_id = Some(show_item.id().clone());
                self.tray_hide_id = Some(hide_item.id().clone());
                self.tray_pause_id = Some(pause_item.id().clone());
                self.tray_resume_id = Some(resume_item.id().clone());
                self.tray_recent_id = Some(recent_item.id().clone());
                self.tray_quit_id = Some(quit_item.id().clone());
                self.tray_icon = Some(tray_icon);
            }
            Err(err) => {
                self.status_message = format!("Tray initialization failed: {err}");
            }
        }
    }

    #[cfg(windows)]
    fn handle_tray_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if self.tray_show_id.as_ref() == Some(&event.id) {
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(ViewportCommand::Focus);
                self.start_in_background = false;
                self.status_message = "NebulaDM restored from the system tray".to_owned();
            } else if self.tray_hide_id.as_ref() == Some(&event.id) {
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
                self.status_message = "NebulaDM is running in the background from the tray".to_owned();
            } else if self.tray_pause_id.as_ref() == Some(&event.id) {
                if let Some(id) = self.active_direct_job_id.or(self.active_torrent_job_id) {
                    self.request_pause_for(id);
                }
            } else if self.tray_resume_id.as_ref() == Some(&event.id) {
                if let Some(id) = self
                    .queue_manager
                    .snapshot()
                    .queue
                    .iter()
                    .find(|item| matches!(item.status, DownloadStatus::Paused | DownloadStatus::Queued))
                    .map(|item| item.id)
                {
                    self.request_resume_for(id);
                }
            } else if self.tray_recent_id.as_ref() == Some(&event.id) {
                if let Some(path) = self.recent_download_targets.first() {
                    let folder = Path::new(path)
                        .parent()
                        .unwrap_or_else(|| Path::new(path));
                    let _ = open_path_in_shell(folder);
                }
            } else if self.tray_quit_id.as_ref() == Some(&event.id) {
                self.quit_requested = true;
            }
        }

        if let Some(tray_icon) = &self.tray_icon {
            let tooltip = if let Some(id) = self.active_direct_job_id {
                format!("NebulaDM\nDirect job #{id} active")
            } else if let Some(id) = self.active_torrent_job_id {
                format!("NebulaDM\nTorrent job #{id} active")
            } else {
                "NebulaDM\nIdle in background".to_owned()
            };
            let _ = tray_icon.set_tooltip(Some(&tooltip));
        }
    }

    #[cfg(windows)]
    fn handle_root_close_to_tray(&mut self, ctx: &egui::Context) {
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if !close_requested || self.quit_requested {
            return;
        }

        ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        self.status_message = "NebulaDM is still running in the tray".to_owned();
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

    fn import_batch_downloads(&mut self) {
        let entries: Vec<String> = self
            .batch_import_sources
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();

        if entries.is_empty() {
            self.status_message = "Paste one or more URLs or magnet links to import".to_owned();
            return;
        }

        let mut added = 0usize;
        for source in entries {
            let kind = infer_download_kind(&source);
            let file_name = infer_display_name_from_source(&source, kind.clone());
            self.queue_manager
                .add_download(file_name, source, kind, false);
            added += 1;
        }

        self.batch_import_sources.clear();
        self.status_message = format!("Imported {added} downloads into the queue");
        let _ = save_snapshot(&self.storage_path, self.queue_manager.snapshot());
        self.save_desktop_state();
    }

    fn poll_clipboard_for_download_links(&mut self) {
        if !self.clipboard_watch_enabled
            || self.last_clipboard_poll_at.elapsed() < Duration::from_millis(900)
        {
            return;
        }
        self.last_clipboard_poll_at = Instant::now();

        let mut clipboard = match Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(_) => return,
        };
        let text = match clipboard.get_text() {
            Ok(text) => text.trim().to_owned(),
            Err(_) => return,
        };
        if text.is_empty() || self.last_clipboard_value.as_deref() == Some(text.as_str()) {
            return;
        }
        self.last_clipboard_value = Some(text.clone());
        if !looks_like_download_source(&text) {
            return;
        }

        self.new_source = text.clone();
        self.new_kind = infer_download_kind(&text);
        self.new_name = infer_display_name_from_source(&text, self.new_kind.clone());
        self.status_message = "Detected a download link from the clipboard".to_owned();
    }

    fn push_notification(&mut self, title: impl Into<String>, body: impl Into<String>) {
        let title = title.into();
        let body = body.into();
        self.notification_serial += 1;
        self.notifications.push(AppNotification {
            id: self.notification_serial,
            title: title.clone(),
            body: body.clone(),
            created_at: Instant::now(),
        });
        if self.notifications.len() > 5 {
            let keep_from = self.notifications.len().saturating_sub(5);
            self.notifications.drain(0..keep_from);
        }
        if self.native_notifications_enabled {
            let _ = show_native_notification(&title, &body);
        }
    }

    fn render_notification_toasts(&mut self, ctx: &egui::Context) {
        self.notifications
            .retain(|notification| notification.created_at.elapsed() < Duration::from_secs(6));
        let Some(notification) = self.notifications.last().cloned() else {
            return;
        };

        let viewport_id = ViewportId::from_hash_of(("download-toast", notification.id));
        let builder = egui::ViewportBuilder::default()
            .with_title("NebulaDM Notification")
            .with_inner_size([340.0, 120.0])
            .with_min_inner_size([320.0, 110.0])
            .with_position(egui::pos2(1200.0, 680.0))
            .with_always_on_top()
            .with_decorations(false)
            .with_resizable(false);

        ctx.show_viewport_immediate(viewport_id, builder, |ctx, class| match class {
            ViewportClass::Immediate | ViewportClass::Embedded => {
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::new()
                            .fill(PANEL)
                            .stroke(egui::Stroke::new(1.0, PANEL_HIGHLIGHT))
                            .corner_radius(18.0)
                            .inner_margin(egui::Margin::symmetric(16, 14)),
                    )
                    .show(ctx, |ui| {
                        ui.label(
                            egui::RichText::new(&notification.title)
                                .strong()
                                .color(BRIGHT_TEXT),
                        );
                        ui.label(egui::RichText::new(&notification.body).color(MUTED_TEXT));
                    });
            }
            _ => {}
        });
    }

    fn render_setup_center(&mut self, ctx: &egui::Context) {
        if !self.show_setup_center {
            return;
        }

        let mut open = self.show_setup_center;
        let mut setup_browser_extension = false;
        let mut register_magnet_links = false;
        let mut enable_run_on_startup = false;
        let mut check_for_updates = false;
        let mut build_installer = false;
        let mut open_onboarding = false;
        egui::Window::new("Setup Center")
            .default_width(560.0)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Guided Setup")
                        .size(22.0)
                        .strong()
                        .color(BRIGHT_TEXT),
                );
                ui.label(
                    egui::RichText::new(
                        "Finish the NebulaDM basics: browser capture, startup, associations, and updates.",
                    )
                    .color(MUTED_TEXT),
                );
                ui.add_space(12.0);
                if ui.button("Setup Browser Extension").clicked() {
                    setup_browser_extension = true;
                }
                if ui.button("Register Magnet Links").clicked() {
                    register_magnet_links = true;
                }
                if ui.button("Enable Run On Startup").clicked() {
                    enable_run_on_startup = true;
                }
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Update Feed URL").color(MUTED_TEXT));
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.update_feed_url)
                            .desired_width(ui.available_width())
                            .hint_text("https://your-domain/releases/windows/update.json"),
                    )
                    .changed()
                {
                    self.save_desktop_state();
                }
                ui.horizontal(|ui| {
                    if ui.button("Check For Updates").clicked() {
                        check_for_updates = true;
                    }
                    if ui.button("Build Windows Installer").clicked() {
                        build_installer = true;
                    }
                    if ui.button("Open Onboarding Guide").clicked() {
                        open_onboarding = true;
                    }
                });
            });
        self.show_setup_center = open;

        if setup_browser_extension {
            self.open_browser_extension_setup();
        }
        if register_magnet_links {
            self.register_magnet_protocol();
        }
        if enable_run_on_startup {
            self.set_run_on_startup(true);
        }
        if check_for_updates {
            self.check_for_updates();
        }
        if build_installer {
            self.build_windows_installer();
        }
        if open_onboarding {
            match write_onboarding_guide_page().and_then(open_in_default_browser) {
                Ok(()) => {
                    self.status_message = "Opened the onboarding guide in your browser".to_owned();
                }
                Err(err) => {
                    self.status_message = format!("Could not open onboarding guide: {err}");
                }
            }
        }
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
        style.spacing.item_spacing = egui::vec2(10.0, 12.0);
        style.spacing.button_padding = egui::vec2(14.0, 10.0);
        style.spacing.indent = 12.0;
        style.visuals = egui::Visuals::dark();
        style.visuals.override_text_color = Some(BRIGHT_TEXT);
        style.visuals.widgets.noninteractive.bg_fill = PANEL;
        style.visuals.widgets.inactive.bg_fill = PANEL_ALT;
        style.visuals.widgets.hovered.bg_fill = PANEL_ALT.linear_multiply(1.08);
        style.visuals.widgets.active.bg_fill = PANEL_ALT.linear_multiply(1.12);
        style.visuals.widgets.open.bg_fill = PANEL_ALT.linear_multiply(1.08);
        style.visuals.window_fill = BACKGROUND;
        style.visuals.panel_fill = BACKGROUND;
        style.visuals.faint_bg_color = PANEL_ALT;
        style.visuals.extreme_bg_color = PANEL_SUBTLE;
        style.visuals.code_bg_color = PANEL_SUBTLE;
        style.visuals.window_stroke = egui::Stroke::new(1.0, OUTLINE);
        style.visuals.hyperlink_color = ACCENT;
        style.visuals.selection.bg_fill = ACCENT.linear_multiply(0.35);
        style.visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, OUTLINE);
        style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT.linear_multiply(0.55));
        style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
        style.visuals.widgets.noninteractive.weak_bg_fill = PANEL_SUBTLE;
        style.visuals.widgets.inactive.weak_bg_fill = PANEL_SUBTLE;
        style.visuals.widgets.hovered.weak_bg_fill = PANEL_ALT;
        style.visuals.widgets.active.weak_bg_fill = PANEL_ALT;
        style.visuals.widgets.inactive.corner_radius = 14.0.into();
        style.visuals.widgets.hovered.corner_radius = 14.0.into();
        style.visuals.widgets.active.corner_radius = 14.0.into();
        style.visuals.widgets.open.corner_radius = 14.0.into();
        ctx.set_style(style);
    }

    fn pill(&self, ui: &mut egui::Ui, text: &str, tint: egui::Color32) {
        egui::Frame::new()
            .fill(tint.linear_multiply(0.18))
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
                    match write_browser_extension_setup_page(&path).and_then(open_in_default_browser)
                    {
                        Ok(_) => {
                            self.status_message =
                                "Opened browser extension setup in your default browser"
                                    .to_owned();
                        }
                        Err(err) => {
                            self.status_message = format!(
                                "Could not open browser extension setup in your browser: {err}"
                            );
                        }
                    }
                }

                #[cfg(not(windows))]
                {
                    self.status_message = format!(
                        "Browser extension folder is available at {}",
                        path.display()
                    );
                }
            }
            None => {
                self.status_message =
                    "Browser extension folder was not found near the app or workspace".to_owned();
            }
        }
    }

    fn check_for_updates(&mut self) {
        let feed_url = self.update_feed_url.trim().to_owned();
        if feed_url.is_empty() {
            self.status_message =
                "Set an update feed URL first so NebulaDM knows where to check".to_owned();
            return;
        }

        match fetch_update_manifest(&feed_url).and_then(download_update_installer_if_newer) {
            Ok(UpdateCheckResult::AlreadyCurrent(version)) => {
                self.status_message = format!("NebulaDM is already up to date ({version})");
            }
            Ok(UpdateCheckResult::InstallerReady { version, installer_path, notes_url }) => {
                self.status_message = format!("Downloaded NebulaDM {version}. Launching installer...");
                let _ = open_path_in_shell(&installer_path);
                if let Some(url) = notes_url {
                    self.status_message.push_str(&format!(" Release notes: {url}"));
                }
                self.push_notification(
                    "Update ready",
                    format!("NebulaDM {version} was downloaded and the installer is ready."),
                );
            }
            Err(err) => {
                self.status_message = format!("Update check failed: {err}");
            }
        }
    }

    fn build_windows_installer(&mut self) {
        match write_windows_installer_script() {
            Ok(script_path) => {
                self.status_message = format!(
                    "Generated Windows installer script at {}",
                    script_path.display()
                );
                let _ = open_path_in_shell(&script_path);
            }
            Err(err) => {
                self.status_message = format!("Installer generation failed: {err}");
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

    fn prepare_direct_request_for_start(
        &mut self,
        id: u64,
        record: &DownloadRecord,
    ) -> Option<DownloadRequest> {
        let snapshot = self.queue_manager.snapshot().clone();
        let mut request = record.request.clone();

        if request.kind != DownloadKind::Direct {
            return Some(request);
        }

        let mut plan = build_direct_download_plan(&request, &snapshot.downloads_root, &snapshot.categories);
        let final_path = PathBuf::from(&plan.final_file_path);

        if final_path.exists() {
            match self.duplicate_strategy {
                DuplicateStrategy::Overwrite => {
                    let _ = fs::remove_file(&plan.final_file_path);
                    let _ = fs::remove_file(&plan.temp_file_path);
                    let _ = fs::remove_file(&plan.metadata_file_path);
                }
                DuplicateStrategy::Skip => {
                    self.queue_manager.fail(id, "Skipped duplicate file");
                    self.push_notification(
                        "Duplicate skipped",
                        format!("{} already exists on disk", record.request.file_name),
                    );
                    self.status_message =
                        format!("Skipped job #{id} because the target file already exists");
                    let _ = save_snapshot(&self.storage_path, self.queue_manager.snapshot());
                    return None;
                }
                DuplicateStrategy::Rename => {
                    let renamed = unique_file_name(&request.file_name, &plan.target.target_folder);
                    request.file_name = renamed;
                    if let Some(existing) = self.queue_manager.get_record_mut(id) {
                        existing.request.file_name = request.file_name.clone();
                    }
                    plan = build_direct_download_plan(
                        &request,
                        &snapshot.downloads_root,
                        &snapshot.categories,
                    );
                    let _ = plan;
                }
            }
        }

        Some(request)
    }

    fn remember_recent_download(&mut self, final_file_path: &str) {
        self.recent_download_targets.retain(|path| path != final_file_path);
        self.recent_download_targets
            .insert(0, final_file_path.to_owned());
        self.recent_download_targets.truncate(5);
    }

    fn run_post_download_action(&self, final_file_path: &str) {
        match self.post_download_action {
            PostDownloadAction::None => {}
            PostDownloadAction::OpenFile => {
                let _ = open_path_in_shell(Path::new(final_file_path));
            }
            PostDownloadAction::OpenFolder => {
                let folder = Path::new(final_file_path)
                    .parent()
                    .unwrap_or_else(|| Path::new(final_file_path));
                let _ = open_path_in_shell(folder);
            }
        }
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

        let Some(request) = self.prepare_direct_request_for_start(id, &record) else {
            return;
        };

        let plan = build_direct_download_plan(
            &request,
            &snapshot.downloads_root,
            &snapshot.categories,
        );
        self.queue_manager.resume(id);
        self.active_direct_download = Some(spawn_direct_download(request, plan));
        self.active_direct_job_id = Some(id);
        self.status_message = format!("Started direct job #{id}");
    }

    fn poll_direct_events(&mut self) {
        let Some(job_id) = self.active_direct_job_id else {
            return;
        };

        let mut finished = false;
        let mut scrub_completed_metadata = false;
        let mut completed_file_path: Option<String> = None;
        let mut failed_message: Option<String> = None;
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
                        completed_file_path = Some(final_file_path.clone());
                        self.status_message =
                            format!("Completed job #{job_id} -> {final_file_path}");
                        finished = true;
                    }
                    DirectDownloadEvent::Failed { message } => {
                        self.queue_manager.fail(job_id, &message);
                        failed_message = Some(message.clone());
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

        if let Some(path) = completed_file_path {
            self.remember_recent_download(&path);
            self.run_post_download_action(&path);
            self.push_notification(
                "Download completed",
                format!("Direct job #{job_id} finished downloading"),
            );
        }

        if let Some(message) = failed_message {
            self.push_notification(
                "Download failed",
                format!("Direct job #{job_id} failed: {message}"),
            );
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
                let preview_plan = plan_download(
                    &payload.clone().into_request(),
                    &self.queue_manager.snapshot().downloads_root,
                    &self.queue_manager.snapshot().categories,
                );
                self.pending_browser_capture = Some(payload);
                self.pending_browser_capture_save_folder = preview_plan.target_folder;
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
            self.push_notification(
                "Torrent completed",
                format!("Torrent job #{job_id} finished"),
            );
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
                        self.push_notification(
                            "Torrent completed",
                            format!("Torrent job #{job_id} finished"),
                        );
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
                        self.push_notification(
                            "Torrent failed",
                            format!("Torrent job #{job_id} failed: {message}"),
                        );
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
            run_on_startup: self.run_on_startup,
            clipboard_watch_enabled: self.clipboard_watch_enabled,
            native_notifications_enabled: self.native_notifications_enabled,
            update_feed_url: self.update_feed_url.clone(),
            duplicate_strategy: self.duplicate_strategy,
            post_download_action: self.post_download_action,
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

enum UpdateCheckResult {
    AlreadyCurrent(String),
    InstallerReady {
        version: String,
        installer_path: PathBuf,
        notes_url: Option<String>,
    },
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

fn sync_run_on_startup(enabled: bool) -> Option<String> {
    #[cfg(windows)]
    {
        match set_startup_registration(enabled) {
            Ok(()) => Some(if enabled {
                "Startup background mode enabled".to_owned()
            } else {
                "Startup background mode disabled".to_owned()
            }),
            Err(err) => Some(format!("Startup registration failed: {err}")),
        }
    }

    #[cfg(not(windows))]
    {
        let _ = enabled;
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

fn write_browser_extension_setup_page(extension_dir: &Path) -> Result<PathBuf, String> {
    let app_state_dir = resolve_app_state_dir();
    fs::create_dir_all(&app_state_dir).map_err(|err| format!("create app state dir failed: {err}"))?;

    let page_path = app_state_dir.join("browser-extension-setup.html");
    let extension_path = html_escape(&extension_dir.display().to_string());
    let page = format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>NebulaDM Browser Extension Setup</title>
    <style>
      :root {{
        color-scheme: light;
        font-family: "Segoe UI", sans-serif;
      }}
      body {{
        margin: 0;
        background:
          radial-gradient(circle at top left, #ffe8bf, transparent 36%),
          linear-gradient(160deg, #f7f2ea, #e7edf4);
        color: #1f2933;
      }}
      main {{
        max-width: 760px;
        margin: 0 auto;
        padding: 32px 20px 56px;
      }}
      .card {{
        background: rgba(255, 255, 255, 0.86);
        border: 1px solid rgba(17, 94, 89, 0.12);
        border-radius: 20px;
        padding: 24px;
        box-shadow: 0 14px 36px rgba(31, 41, 51, 0.08);
      }}
      h1 {{
        margin: 0 0 8px;
        font-size: 28px;
      }}
      p, li {{
        line-height: 1.55;
      }}
      code {{
        display: block;
        margin-top: 10px;
        padding: 12px 14px;
        border-radius: 12px;
        background: #0f1720;
        color: #d9e2ec;
        word-break: break-all;
      }}
      ol {{
        padding-left: 20px;
      }}
    </style>
  </head>
  <body>
    <main>
      <div class="card">
        <h1>NebulaDM Browser Extension Setup</h1>
        <p>Load the NebulaDM browser extension as an unpacked extension in your Chromium-based browser.</p>
        <ol>
          <li>Open your browser's extensions page.</li>
          <li>Turn on Developer mode.</li>
          <li>Choose <strong>Load unpacked</strong>.</li>
          <li>Select this folder:</li>
        </ol>
        <code>{extension_path}</code>
        <p>Chrome usually uses <code>chrome://extensions</code> and Edge usually uses <code>edge://extensions</code>.</p>
      </div>
    </main>
  </body>
</html>"#
    );

    fs::write(&page_path, page).map_err(|err| format!("write setup page failed: {err}"))?;
    Ok(page_path)
}

#[cfg(windows)]
fn open_in_default_browser(page_path: PathBuf) -> Result<(), String> {
    open_windows_shell_target(&page_path.display().to_string(), "launch browser")
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn looks_like_download_source(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ftp://")
        || lower.starts_with("magnet:")
}

fn infer_display_name_from_source(source: &str, kind: DownloadKind) -> String {
    if kind == DownloadKind::Torrent {
        return infer_magnet_display_name(source).unwrap_or_else(|| "torrent-download.torrent".to_owned());
    }

    source
        .split('?')
        .next()
        .and_then(|path| path.rsplit('/').next())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("download.bin")
        .to_owned()
}

fn unique_file_name(file_name: &str, target_folder: &str) -> String {
    let path = Path::new(file_name);
    let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("download");
    let ext = path.extension().and_then(|value| value.to_str());
    for index in 1..1000 {
        let candidate = match ext {
            Some(ext) if !ext.is_empty() => format!("{stem} ({index}).{ext}"),
            _ => format!("{stem} ({index})"),
        };
        if !Path::new(target_folder).join(&candidate).exists() {
            return candidate;
        }
    }
    file_name.to_owned()
}

fn write_onboarding_guide_page() -> Result<PathBuf, String> {
    let app_state_dir = resolve_app_state_dir();
    fs::create_dir_all(&app_state_dir).map_err(|err| format!("create app state dir failed: {err}"))?;
    let page_path = app_state_dir.join("onboarding-guide.html");
    let page = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>NebulaDM Onboarding</title>
<style>body{font-family:"Segoe UI",sans-serif;background:#111114;color:#f4f6fa;margin:0}main{max-width:860px;margin:0 auto;padding:32px 20px}section{background:#212127;border:1px solid #424753;border-radius:18px;padding:24px;box-shadow:0 16px 40px rgba(0,0,0,.28)}h1{margin:0 0 8px}li{margin:10px 0}code{background:#1a1a20;padding:2px 8px;border-radius:8px}</style>
</head><body><main><section><h1>NebulaDM First-Run Guide</h1><ol><li>Launch NebulaDM and keep it running in the tray for browser handoff.</li><li>Use <strong>Setup Browser Extension</strong> to open the extension onboarding page.</li><li>Use <strong>Register Magnet Links</strong> so magnet links open in NebulaDM.</li><li>Enable <strong>Run on startup in background</strong> if you want IDM-style always-ready behavior.</li><li>Set an <strong>Update Feed URL</strong> in the setup center so NebulaDM can download newer installers automatically.</li><li>Use the generated Inno Setup script in <code>dist\installer\NebulaDM.iss</code> to create a proper Windows installer.</li></ol></section></main></body></html>"#;
    fs::write(&page_path, page).map_err(|err| format!("write onboarding guide failed: {err}"))?;
    Ok(page_path)
}

fn fetch_update_manifest(feed_url: &str) -> Result<UpdateManifest, String> {
    reqwest::blocking::get(feed_url)
        .map_err(|err| format!("request failed: {err}"))?
        .error_for_status()
        .map_err(|err| format!("update feed error: {err}"))?
        .json::<UpdateManifest>()
        .map_err(|err| format!("manifest parse failed: {err}"))
}

fn download_update_installer_if_newer(manifest: UpdateManifest) -> Result<UpdateCheckResult, String> {
    let current =
        Version::parse(env!("CARGO_PKG_VERSION")).map_err(|err| format!("current version parse failed: {err}"))?;
    let available =
        Version::parse(&manifest.version).map_err(|err| format!("update version parse failed: {err}"))?;
    if available <= current {
        return Ok(UpdateCheckResult::AlreadyCurrent(current.to_string()));
    }

    let updates_dir = resolve_app_state_dir().join("updates");
    fs::create_dir_all(&updates_dir).map_err(|err| format!("create updates dir failed: {err}"))?;
    let installer_path = updates_dir.join(format!("NebulaDM-setup-{}.exe", manifest.version));
    let bytes = reqwest::blocking::get(&manifest.installer_url)
        .map_err(|err| format!("installer download failed: {err}"))?
        .error_for_status()
        .map_err(|err| format!("installer response failed: {err}"))?
        .bytes()
        .map_err(|err| format!("read installer payload failed: {err}"))?;
    fs::write(&installer_path, &bytes).map_err(|err| format!("write installer failed: {err}"))?;

    Ok(UpdateCheckResult::InstallerReady {
        version: manifest.version,
        installer_path,
        notes_url: manifest.notes_url,
    })
}

fn write_windows_installer_script() -> Result<PathBuf, String> {
    let repo_root = std::env::current_dir().map_err(|err| format!("current dir failed: {err}"))?;
    let installer_dir = repo_root.join("dist").join("installer");
    fs::create_dir_all(&installer_dir).map_err(|err| format!("create installer dir failed: {err}"))?;
    let iss_path = installer_dir.join("NebulaDM.iss");
    let content = format!(
        r#"; Generated by NebulaDM
[Setup]
AppName=NebulaDM
AppVersion={}
DefaultDirName={{autopf}}\NebulaDM
DefaultGroupName=NebulaDM
OutputDir={}
OutputBaseFilename=NebulaDM-Setup
Compression=lzma
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={{app}}\NebulaDM.exe

[Files]
Source: "{}"; DestDir: "{{app}}"; DestName: "NebulaDM.exe"; Flags: ignoreversion
Source: "{}\*"; DestDir: "{{app}}\browser-extension"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{{group}}\NebulaDM"; Filename: "{{app}}\NebulaDM.exe"
Name: "{{group}}\Uninstall NebulaDM"; Filename: "{{uninstallexe}}"

[Run]
Filename: "{{app}}\NebulaDM.exe"; Description: "Launch NebulaDM"; Flags: nowait postinstall skipifsilent
"#,
        env!("CARGO_PKG_VERSION"),
        html_escape(&installer_dir.display().to_string()),
        html_escape(&repo_root.join("target-release-desktop").join("release").join("desktop.exe").display().to_string()),
        html_escape(&repo_root.join("extensions").join("browser").display().to_string()),
    );
    fs::write(&iss_path, content).map_err(|err| format!("write installer script failed: {err}"))?;
    Ok(iss_path)
}

fn show_native_notification(title: &str, body: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        Toast::new(Toast::POWERSHELL_APP_ID)
            .title(title)
            .text1(body)
            .sound(Some(Sound::Default))
            .duration(ToastDuration::Short)
            .show()
            .map_err(|err| format!("toast failed: {err}"))
    }

    #[cfg(not(windows))]
    {
        let _ = (title, body);
        Ok(())
    }
}

fn open_path_in_shell(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        open_windows_shell_target(&path.display().to_string(), "launch shell")
    }

    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(windows)]
fn open_windows_shell_target(target: &str, action_label: &str) -> Result<(), String> {
    std::process::Command::new("cmd")
        .args(["/c", "start", "", target])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("{action_label} failed: {err}"))
}

#[cfg(windows)]
fn set_startup_registration(enabled: bool) -> Result<(), String> {
    let exe_path =
        std::env::current_exe().map_err(|err| format!("current exe lookup failed: {err}"))?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = hkcu
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|err| format!("open startup run key failed: {err}"))?
        .0;

    if enabled {
        let command = format!("\"{}\" --background", exe_path.display());
        run_key
            .set_value("NebulaDM", &command)
            .map_err(|err| format!("set startup entry failed: {err}"))?;
    } else {
        let _ = run_key.delete_value("NebulaDM");
    }

    Ok(())
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
