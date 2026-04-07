use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};
use std::process::{Command, Stdio};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use reqwest::blocking::Client;
use reqwest::header::{
    ACCEPT, ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, COOKIE, ETAG, HeaderValue,
    LAST_MODIFIED, ORIGIN, RANGE, REFERER, USER_AGENT,
};
use serde::{Deserialize, Serialize};
use shared::{DownloadCategory, DownloadPlan, DownloadRequest};

use crate::planner::plan_download;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectDownloadPlan {
    pub target: DownloadPlan,
    pub final_file_path: String,
    pub temp_file_path: String,
    pub metadata_file_path: String,
    pub supports_resume: bool,
    pub chunk_size_bytes: u64,
    pub parallel_connections: u32,
    pub max_retry_attempts: u32,
    pub retry_backoff_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResumeMetadata {
    pub source_url: String,
    pub final_file_path: String,
    pub temp_file_path: String,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub chunks: Vec<ChunkProgress>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkProgress {
    pub index: u32,
    pub start_byte: u64,
    pub end_byte: Option<u64>,
    pub downloaded_bytes: u64,
}

pub fn build_direct_download_plan(
    request: &DownloadRequest,
    downloads_root: &str,
    categories: &[DownloadCategory],
) -> DirectDownloadPlan {
    let target = plan_download(request, downloads_root, categories);
    let safe_file_name = sanitize_file_name(&request.file_name);
    let final_file_path = format!("{}/{}", target.target_folder, safe_file_name);
    let temp_file_path = format!("{final_file_path}.part");
    let metadata_file_path = format!("{final_file_path}.json");

    DirectDownloadPlan {
        target,
        final_file_path,
        temp_file_path,
        metadata_file_path,
        supports_resume: true,
        chunk_size_bytes: 4 * 1024 * 1024,
        parallel_connections: 4,
        max_retry_attempts: 4,
        retry_backoff_ms: 1200,
    }
}

pub fn create_resume_metadata(
    request: &DownloadRequest,
    plan: &DirectDownloadPlan,
    total_bytes: Option<u64>,
) -> ResumeMetadata {
    let chunks = total_bytes
        .map(|total| build_chunk_progress(total, plan.chunk_size_bytes, plan.parallel_connections))
        .filter(|chunks| !chunks.is_empty())
        .unwrap_or_else(|| {
            vec![ChunkProgress {
                index: 0,
                start_byte: 0,
                end_byte: None,
                downloaded_bytes: 0,
            }]
        });

    ResumeMetadata {
        source_url: request.source.clone(),
        final_file_path: plan.final_file_path.clone(),
        temp_file_path: plan.temp_file_path.clone(),
        total_bytes,
        downloaded_bytes: 0,
        etag: None,
        last_modified: None,
        chunks,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DirectDownloadEvent {
    Started {
        total_bytes: Option<u64>,
    },
    Progress {
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        bytes_per_second: f64,
    },
    Retrying {
        attempt: u32,
        max_attempts: u32,
        wait_ms: u64,
        message: String,
    },
    Completed {
        final_file_path: String,
        total_bytes: u64,
    },
    Failed {
        message: String,
    },
    Paused {
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
}

#[derive(Debug)]
pub enum DirectDownloadCommand {
    Pause,
}

pub struct ActiveDirectDownload {
    pub events: Receiver<DirectDownloadEvent>,
    pub commands: Sender<DirectDownloadCommand>,
}

pub fn spawn_direct_download(
    request: DownloadRequest,
    plan: DirectDownloadPlan,
) -> ActiveDirectDownload {
    let (sender, receiver) = mpsc::channel();
    let (command_sender, command_receiver) = mpsc::channel();

    thread::spawn(move || {
        if let Err(err) = run_direct_download(&request, &plan, &sender, &command_receiver) {
            let _ = sender.send(DirectDownloadEvent::Failed { message: err });
        }
    });

    ActiveDirectDownload {
        events: receiver,
        commands: command_sender,
    }
}

fn run_direct_download(
    request: &DownloadRequest,
    plan: &DirectDownloadPlan,
    sender: &mpsc::Sender<DirectDownloadEvent>,
    command_receiver: &Receiver<DirectDownloadCommand>,
) -> Result<(), String> {
    if request.use_yt_dlp {
        return run_yt_dlp_download(request, plan, sender, command_receiver);
    }

    if request.secondary_source.is_some() {
        return run_adaptive_download(request, plan, sender, command_receiver);
    }

    let final_path = Path::new(&plan.final_file_path);
    let temp_path = Path::new(&plan.temp_file_path);
    let metadata_path = Path::new(&plan.metadata_file_path);

    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create dir failed: {err}"))?;
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|err| format!("http client failed: {err}"))?;

    let existing_metadata = load_resume_metadata(metadata_path);
    let existing_bytes = fs::metadata(temp_path).map(|meta| meta.len()).unwrap_or(0);
    let session = start_or_resume_session(
        &client,
        request,
        plan,
        existing_metadata.as_ref(),
        existing_bytes,
        false,
    )?;

    let mut metadata = if session.resumed {
        existing_metadata
            .unwrap_or_else(|| create_resume_metadata(request, plan, session.remote_total_bytes))
    } else {
        create_resume_metadata(request, plan, session.remote_total_bytes)
    };

    if !session.resumed && existing_bytes > 0 {
        let _ = fs::remove_file(temp_path);
    }

    metadata.total_bytes = session.remote_total_bytes;
    metadata.downloaded_bytes = if session.resumed { existing_bytes } else { 0 };
    metadata.etag = session.etag.clone().or_else(|| metadata.etag.clone());
    metadata.last_modified = session
        .last_modified
        .clone()
        .or_else(|| metadata.last_modified.clone());
    if metadata.chunks.len() == 1 {
        if let Some(chunk) = metadata.chunks.first_mut() {
            chunk.downloaded_bytes = metadata.downloaded_bytes;
            chunk.end_byte = metadata.total_bytes.map(|value| value.saturating_sub(1));
        }
    }

    save_resume_metadata(metadata_path, &metadata)?;
    let _ = sender.send(DirectDownloadEvent::Started {
        total_bytes: metadata.total_bytes,
    });

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .open(temp_path)
        .map_err(|err| format!("open temp file failed: {err}"))?;
    if session.resumed {
        file.seek(SeekFrom::End(0))
            .map_err(|err| format!("seek failed: {err}"))?;
    } else {
        file.set_len(0)
            .map_err(|err| format!("reset temp file failed: {err}"))?;
    }
    if should_use_parallel_download(&session, existing_bytes, &metadata, plan) {
        file.set_len(metadata.total_bytes.unwrap_or(0))
            .map_err(|err| format!("preallocate temp file failed: {err}"))?;
        drop(file);
        return run_parallel_download(
            &client,
            request,
            plan,
            sender,
            command_receiver,
            metadata_path,
            temp_path,
            final_path,
            metadata,
        );
    }

    let mut buffer = [0_u8; 64 * 1024];
    let mut downloaded_bytes = metadata.downloaded_bytes;
    let started_at = Instant::now();
    let mut attempt = 0;
    let mut response = session.response;

    loop {
        match command_receiver.try_recv() {
            Ok(DirectDownloadCommand::Pause) => {
                file.flush().map_err(|err| format!("flush failed: {err}"))?;
                save_resume_metadata(metadata_path, &metadata)?;
                let _ = sender.send(DirectDownloadEvent::Paused {
                    downloaded_bytes,
                    total_bytes: metadata.total_bytes,
                });
                return Ok(());
            }
            Err(TryRecvError::Disconnected) => {
                return Err("download control disconnected".to_owned());
            }
            Err(TryRecvError::Empty) => {}
        }

        let read = match response.read(&mut buffer) {
            Ok(value) => value,
            Err(err) => {
                if attempt >= plan.max_retry_attempts {
                    return Err(format!("read failed after retries: {err}"));
                }

                attempt += 1;
                let wait_ms = plan.retry_backoff_ms * u64::from(attempt);
                let _ = sender.send(DirectDownloadEvent::Retrying {
                    attempt,
                    max_attempts: plan.max_retry_attempts,
                    wait_ms,
                    message: err.to_string(),
                });
                thread::sleep(Duration::from_millis(wait_ms));
                file.flush()
                    .map_err(|flush_err| format!("flush failed: {flush_err}"))?;
                save_resume_metadata(metadata_path, &metadata)?;
                let retry_session = start_or_resume_session(
                    &client,
                    request,
                    plan,
                    Some(&metadata),
                    downloaded_bytes,
                    true,
                )?;
                metadata.total_bytes = retry_session.remote_total_bytes;
                metadata.etag = retry_session.etag.or(metadata.etag);
                metadata.last_modified = retry_session.last_modified.or(metadata.last_modified);
                response = retry_session.response;
                continue;
            }
        };

        if read == 0 {
            break;
        }

        attempt = 0;
        file.write_all(&buffer[..read])
            .map_err(|err| format!("write failed: {err}"))?;

        downloaded_bytes += read as u64;
        metadata.downloaded_bytes = downloaded_bytes;
        if let Some(chunk) = metadata.chunks.first_mut() {
            chunk.downloaded_bytes = downloaded_bytes;
        }

        save_resume_metadata(metadata_path, &metadata)?;

        let elapsed = started_at.elapsed().as_secs_f64().max(0.1);
        let _ = sender.send(DirectDownloadEvent::Progress {
            downloaded_bytes,
            total_bytes: metadata.total_bytes,
            bytes_per_second: downloaded_bytes as f64 / elapsed,
        });
    }

    file.flush().map_err(|err| format!("flush failed: {err}"))?;
    fs::rename(temp_path, final_path).map_err(|err| format!("finalize failed: {err}"))?;
    let _ = fs::remove_file(metadata_path);
    let _ = sender.send(DirectDownloadEvent::Completed {
        final_file_path: plan.final_file_path.clone(),
        total_bytes: downloaded_bytes,
    });
    Ok(())
}

fn run_yt_dlp_download(
    request: &DownloadRequest,
    plan: &DirectDownloadPlan,
    sender: &mpsc::Sender<DirectDownloadEvent>,
    command_receiver: &Receiver<DirectDownloadCommand>,
) -> Result<(), String> {
    let final_path = Path::new(&plan.final_file_path);
    let target_dir = final_path
        .parent()
        .ok_or_else(|| format!("invalid output directory for {}", plan.final_file_path))?;
    fs::create_dir_all(target_dir).map_err(|err| format!("create dir failed: {err}"))?;

    let yt_dlp_path = resolve_yt_dlp_path().ok_or_else(|| {
        "A bundled yt-dlp binary was not found. Place yt-dlp.exe next to NebulaDM.exe or in tools\\yt-dlp.exe.".to_owned()
    })?;

    let mut command = Command::new(&yt_dlp_path);
    command.current_dir(target_dir);
    command
        .arg("--no-playlist")
        .arg("--no-part")
        .arg("--newline")
        .arg("--windows-filenames")
        .arg("--force-overwrites")
        .arg("--format")
        .arg("bv*+ba/b")
        .arg("--output")
        .arg("%(title).200B [%(id)s].%(ext)s")
        .arg("--restrict-filenames")
        .arg("--no-warnings")
        .arg("--merge-output-format")
        .arg("mp4");
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    if let Some(ffmpeg_path) = resolve_ffmpeg_path() {
        command.arg("--ffmpeg-location").arg(ffmpeg_path);
    }

    if let Some(referrer) = &request.referrer {
        command.arg("--referer").arg(referrer);
    }
    if let Some(user_agent) = &request.user_agent {
        command.arg("--user-agent").arg(user_agent);
    }
    if let Some(cookie_header) = &request.cookie_header {
        command.arg("--add-header").arg(format!("Cookie:{cookie_header}"));
    }
    if let Some(origin) = &request.origin {
        command.arg("--add-header").arg(format!("Origin:{origin}"));
    }

    command.arg(&request.source);

    let mut child = command
        .spawn()
        .map_err(|err| format!("yt-dlp launch failed at {}: {err}", yt_dlp_path.display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "yt-dlp did not expose stdout".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "yt-dlp did not expose stderr".to_owned())?;
    let (line_sender, line_receiver) = mpsc::channel::<String>();
    let stdout_sender = line_sender.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let _ = stdout_sender.send(line);
        }
    });
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let _ = line_sender.send(line);
        }
    });

    let _ = sender.send(DirectDownloadEvent::Started { total_bytes: None });
    let started_at = std::time::SystemTime::now();
    let mut last_progress_total = None;

    loop {
        while let Ok(line) = line_receiver.try_recv() {
            if let Some((downloaded_bytes, total_bytes, bytes_per_second)) =
                parse_yt_dlp_progress_line(&line)
            {
                last_progress_total = total_bytes.or(last_progress_total);
                let _ = sender.send(DirectDownloadEvent::Progress {
                    downloaded_bytes,
                    total_bytes: total_bytes.or(last_progress_total),
                    bytes_per_second,
                });
            }
        }

        match command_receiver.try_recv() {
            Ok(DirectDownloadCommand::Pause) => {
                let _ = child.kill();
                let _ = sender.send(DirectDownloadEvent::Paused {
                    downloaded_bytes: 0,
                    total_bytes: None,
                });
                return Ok(());
            }
            Err(TryRecvError::Disconnected) => {
                let _ = child.kill();
                return Err("download control disconnected".to_owned());
            }
            Err(TryRecvError::Empty) => {}
        }

        match child.try_wait().map_err(|err| format!("yt-dlp wait failed: {err}"))? {
            Some(status) => {
                if status.success() {
                    let completed_path =
                        find_latest_downloaded_media_file(target_dir, started_at).unwrap_or_else(|| {
                            plan.final_file_path.clone()
                        });
                    let total_bytes = fs::metadata(&completed_path).map(|meta| meta.len()).unwrap_or(0);
                    let _ = sender.send(DirectDownloadEvent::Completed {
                        final_file_path: completed_path,
                        total_bytes,
                    });
                    return Ok(());
                }

                cleanup_recent_media_files(target_dir, started_at);
                return Err(format!("yt-dlp failed with status {status}"));
            }
            None => {
                thread::sleep(Duration::from_millis(150));
            }
        }
    }
}

fn parse_yt_dlp_progress_line(line: &str) -> Option<(u64, Option<u64>, f64)> {
    let trimmed = line.trim();
    if !trimmed.starts_with("[download]") || !trimmed.contains('%') || !trimmed.contains(" of ") {
        return None;
    }

    let percent_end = trimmed.find('%')?;
    let percent_start = trimmed[..percent_end].rfind(' ')? + 1;
    let percent_value: f64 = trimmed[percent_start..percent_end].trim().parse().ok()?;

    let total_start = trimmed.find(" of ")? + 4;
    let total_end = trimmed[total_start..]
        .find(" at ")
        .map(|index| total_start + index)
        .or_else(|| trimmed[total_start..].find(" ETA ").map(|index| total_start + index))
        .unwrap_or(trimmed.len());
    let total_bytes = parse_human_size(trimmed[total_start..total_end].trim())?;
    let downloaded_bytes = ((percent_value / 100.0) * total_bytes as f64) as u64;

    let bytes_per_second = trimmed
        .find(" at ")
        .and_then(|start| {
            let value_start = start + 4;
            let value_end = trimmed[value_start..]
                .find(" ETA ")
                .map(|index| value_start + index)
                .unwrap_or(trimmed.len());
            parse_human_speed(trimmed[value_start..value_end].trim())
        })
        .unwrap_or(0.0);

    Some((downloaded_bytes, Some(total_bytes), bytes_per_second))
}

fn parse_human_speed(value: &str) -> Option<f64> {
    let cleaned = value.trim();
    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("unknown b/s") {
        return None;
    }
    let bytes = parse_human_size(cleaned.strip_suffix("/s").unwrap_or(cleaned))?;
    Some(bytes as f64)
}

fn parse_human_size(value: &str) -> Option<u64> {
    let cleaned = value.trim();
    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("unknown") {
        return None;
    }

    let mut split_index = 0;
    for (index, ch) in cleaned.char_indices() {
        if !(ch.is_ascii_digit() || ch == '.') {
            split_index = index;
            break;
        }
    }
    if split_index == 0 {
        split_index = cleaned.len();
    }

    let number: f64 = cleaned[..split_index].trim().parse().ok()?;
    let unit = cleaned[split_index..].trim().to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "b" | "" => 1.0,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "kb" => 1000.0,
        "mb" => 1000.0 * 1000.0,
        "gb" => 1000.0 * 1000.0 * 1000.0,
        "tb" => 1000.0 * 1000.0 * 1000.0 * 1000.0,
        _ => return None,
    };
    Some((number * multiplier) as u64)
}

fn find_latest_downloaded_media_file(
    directory: &Path,
    started_at: std::time::SystemTime,
) -> Option<String> {
    let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;
    let entries = fs::read_dir(directory).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if !matches!(
            extension.as_str(),
            "mp4" | "mkv" | "webm" | "mov" | "m4v" | "mp3" | "m4a" | "wav" | "aac"
        ) {
            continue;
        }

        let modified = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())?;
        if modified < started_at {
            continue;
        }

        match &latest {
            Some((latest_modified, _)) if modified <= *latest_modified => {}
            _ => latest = Some((modified, path)),
        }
    }

    latest.map(|(_, path)| path.display().to_string())
}

fn cleanup_recent_media_files(directory: &Path, started_at: std::time::SystemTime) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if !matches!(
            extension.as_str(),
            "mp4" | "mkv" | "webm" | "mov" | "m4v" | "mp3" | "m4a" | "wav" | "aac"
        ) {
            continue;
        }

        let modified = match entry.metadata().ok().and_then(|metadata| metadata.modified().ok()) {
            Some(value) => value,
            None => continue,
        };
        if modified >= started_at {
            let _ = fs::remove_file(path);
        }
    }
}

fn run_adaptive_download(
    request: &DownloadRequest,
    plan: &DirectDownloadPlan,
    sender: &mpsc::Sender<DirectDownloadEvent>,
    command_receiver: &Receiver<DirectDownloadCommand>,
) -> Result<(), String> {
    let final_path = Path::new(&plan.final_file_path);
    let video_temp_path = format!("{}.video.part", plan.temp_file_path);
    let audio_temp_path = format!("{}.audio.part", plan.temp_file_path);
    let video_temp = Path::new(&video_temp_path);
    let audio_temp = Path::new(&audio_temp_path);

    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create dir failed: {err}"))?;
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|err| format!("http client failed: {err}"))?;

    let video_total = probe_content_length(&client, request, &request.source)?;
    let audio_source = request
        .secondary_source
        .clone()
        .ok_or_else(|| "adaptive download missing audio source".to_owned())?;
    let audio_total = probe_content_length(&client, request, &audio_source)?;
    let total_bytes = match (video_total, audio_total) {
        (Some(video), Some(audio)) => Some(video + audio),
        _ => None,
    };

    let _ = sender.send(DirectDownloadEvent::Started { total_bytes });

    let started_at = Instant::now();
    let video_downloaded = match download_stream_to_file(
        &client,
        request,
        &request.source,
        video_temp,
        0,
        total_bytes,
        sender,
        command_receiver,
        started_at,
    ) {
        Ok(bytes) => bytes,
        Err(message) if message == "adaptive download paused" => return Ok(()),
        Err(message) => return Err(message),
    };
    let audio_downloaded = match download_stream_to_file(
        &client,
        request,
        &audio_source,
        audio_temp,
        video_downloaded,
        total_bytes,
        sender,
        command_receiver,
        started_at,
    ) {
        Ok(bytes) => bytes,
        Err(message) if message == "adaptive download paused" => return Ok(()),
        Err(message) => return Err(message),
    };

    merge_adaptive_streams(video_temp, audio_temp, final_path)?;
    let _ = fs::remove_file(video_temp);
    let _ = fs::remove_file(audio_temp);
    let _ = fs::remove_file(&plan.metadata_file_path);
    let _ = sender.send(DirectDownloadEvent::Completed {
        final_file_path: plan.final_file_path.clone(),
        total_bytes: video_downloaded + audio_downloaded,
    });
    Ok(())
}

#[derive(Debug)]
enum ParallelChunkEvent {
    Progress {
        chunk_index: u32,
        downloaded_bytes: u64,
    },
    Retrying {
        attempt: u32,
        max_attempts: u32,
        wait_ms: u64,
        message: String,
    },
    Completed,
    Failed(String),
}

fn run_parallel_download(
    client: &Client,
    request: &DownloadRequest,
    plan: &DirectDownloadPlan,
    sender: &mpsc::Sender<DirectDownloadEvent>,
    command_receiver: &Receiver<DirectDownloadCommand>,
    metadata_path: &Path,
    temp_path: &Path,
    final_path: &Path,
    mut metadata: ResumeMetadata,
) -> Result<(), String> {
    let pause_flag = Arc::new(AtomicBool::new(false));
    let (chunk_sender, chunk_receiver) = mpsc::channel();
    let started_at = Instant::now();

    for chunk in metadata.chunks.clone() {
        let worker_sender = chunk_sender.clone();
        let worker_client = client.clone();
        let worker_request = request.clone();
        let temp_file_path = temp_path.to_path_buf();
        let pause_flag = pause_flag.clone();
        let max_retry_attempts = plan.max_retry_attempts;
        let retry_backoff_ms = plan.retry_backoff_ms;

        thread::spawn(move || {
            let _ = download_chunk_range(
                &worker_client,
                &worker_request,
                &temp_file_path,
                chunk,
                &pause_flag,
                max_retry_attempts,
                retry_backoff_ms,
                &worker_sender,
            );
        });
    }
    drop(chunk_sender);

    let chunk_count = metadata.chunks.len();
    let mut completed_chunks = 0usize;
    let mut chunk_totals: Vec<u64> = metadata.chunks.iter().map(|c| c.downloaded_bytes).collect();

    loop {
        match command_receiver.try_recv() {
            Ok(DirectDownloadCommand::Pause) => {
                pause_flag.store(true, Ordering::Relaxed);
                save_resume_metadata(metadata_path, &metadata)?;
                let _ = sender.send(DirectDownloadEvent::Paused {
                    downloaded_bytes: metadata.downloaded_bytes,
                    total_bytes: metadata.total_bytes,
                });
                return Ok(());
            }
            Err(TryRecvError::Disconnected) => {
                pause_flag.store(true, Ordering::Relaxed);
                return Err("download control disconnected".to_owned());
            }
            Err(TryRecvError::Empty) => {}
        }

        match chunk_receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(ParallelChunkEvent::Progress {
                chunk_index,
                downloaded_bytes,
            }) => {
                if let Some(chunk) = metadata.chunks.get_mut(chunk_index as usize) {
                    chunk.downloaded_bytes = downloaded_bytes;
                }
                if let Some(total) = chunk_totals.get_mut(chunk_index as usize) {
                    *total = downloaded_bytes;
                }
                metadata.downloaded_bytes = chunk_totals.iter().sum();
                save_resume_metadata(metadata_path, &metadata)?;
                let elapsed = started_at.elapsed().as_secs_f64().max(0.1);
                let _ = sender.send(DirectDownloadEvent::Progress {
                    downloaded_bytes: metadata.downloaded_bytes,
                    total_bytes: metadata.total_bytes,
                    bytes_per_second: metadata.downloaded_bytes as f64 / elapsed,
                });
            }
            Ok(ParallelChunkEvent::Retrying {
                attempt,
                max_attempts,
                wait_ms,
                message,
            }) => {
                let _ = sender.send(DirectDownloadEvent::Retrying {
                    attempt,
                    max_attempts,
                    wait_ms,
                    message,
                });
            }
            Ok(ParallelChunkEvent::Completed) => {
                completed_chunks += 1;
                if completed_chunks >= chunk_count {
                    break;
                }
            }
            Ok(ParallelChunkEvent::Failed(message)) => {
                pause_flag.store(true, Ordering::Relaxed);
                return Err(message);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("parallel chunk workers disconnected".to_owned());
            }
        }
    }

    fs::rename(temp_path, final_path).map_err(|err| format!("finalize failed: {err}"))?;
    let _ = fs::remove_file(metadata_path);
    let _ = sender.send(DirectDownloadEvent::Completed {
        final_file_path: plan.final_file_path.clone(),
        total_bytes: metadata.downloaded_bytes,
    });
    Ok(())
}

struct DirectDownloadSession {
    response: reqwest::blocking::Response,
    remote_total_bytes: Option<u64>,
    resumed: bool,
    etag: Option<String>,
    last_modified: Option<String>,
}

fn start_or_resume_session(
    client: &Client,
    request: &DownloadRequest,
    plan: &DirectDownloadPlan,
    existing_metadata: Option<&ResumeMetadata>,
    existing_bytes: u64,
    require_resume: bool,
) -> Result<DirectDownloadSession, String> {
    let mut request_builder = build_direct_request(client, request);
    if existing_bytes > 0 || requires_browser_context(&request.source) {
        request_builder = request_builder.header(RANGE, format!("bytes={existing_bytes}-"));
    }

    let response = request_builder
        .send()
        .map_err(|err| format!("request failed: {err}"))?
        .error_for_status()
        .map_err(|err| format!("bad response: {err}"))?;

    let supports_range = response
        .headers()
        .get(ACCEPT_RANGES)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.contains("bytes"))
        .unwrap_or(false)
        || response.headers().get(CONTENT_RANGE).is_some();
    let resumed = existing_bytes > 0 && plan.supports_resume && supports_range;

    if require_resume && existing_bytes > 0 && !resumed {
        return Err("server no longer supports ranged resume for this download".to_owned());
    }

    let etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .or_else(|| existing_metadata.and_then(|metadata| metadata.etag.clone()));
    let last_modified = response
        .headers()
        .get(LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .or_else(|| existing_metadata.and_then(|metadata| metadata.last_modified.clone()));
    let remote_total_bytes = parse_total_bytes(response.headers(), existing_bytes);

    Ok(DirectDownloadSession {
        response,
        remote_total_bytes,
        resumed,
        etag,
        last_modified,
    })
}

fn download_chunk_range(
    client: &Client,
    request: &DownloadRequest,
    temp_path: &Path,
    chunk: ChunkProgress,
    pause_flag: &AtomicBool,
    max_retry_attempts: u32,
    retry_backoff_ms: u64,
    sender: &Sender<ParallelChunkEvent>,
) -> Result<(), String> {
    let chunk_length = chunk
        .end_byte
        .map(|end| end.saturating_sub(chunk.start_byte) + 1)
        .ok_or_else(|| "parallel chunk missing end byte".to_owned())?;
    let mut local_downloaded = chunk.downloaded_bytes;
    let mut attempt = 0;

    while local_downloaded < chunk_length {
        if pause_flag.load(Ordering::Relaxed) {
            return Ok(());
        }

        let range_start = chunk.start_byte + local_downloaded;
        let range_end = chunk.end_byte.unwrap_or(range_start);
        let mut response = build_direct_request(client, request)
            .header(RANGE, format!("bytes={range_start}-{range_end}"))
            .send()
            .map_err(|err| format!("chunk request failed: {err}"))?
            .error_for_status()
            .map_err(|err| format!("chunk response failed: {err}"))?;

        let mut file = OpenOptions::new()
            .write(true)
            .open(temp_path)
            .map_err(|err| format!("open chunk temp file failed: {err}"))?;
        file.seek(SeekFrom::Start(range_start))
            .map_err(|err| format!("chunk seek failed: {err}"))?;

        let mut buffer = [0_u8; 64 * 1024];
        loop {
            if pause_flag.load(Ordering::Relaxed) {
                file.flush()
                    .map_err(|err| format!("chunk flush failed: {err}"))?;
                return Ok(());
            }

            match response.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    attempt = 0;
                    file.write_all(&buffer[..read])
                        .map_err(|err| format!("chunk write failed: {err}"))?;
                    local_downloaded += read as u64;
                    let _ = sender.send(ParallelChunkEvent::Progress {
                        chunk_index: chunk.index,
                        downloaded_bytes: local_downloaded,
                    });
                }
                Err(err) => {
                    if attempt >= max_retry_attempts {
                        let _ = sender.send(ParallelChunkEvent::Failed(format!(
                            "chunk {} failed after retries: {err}",
                            chunk.index
                        )));
                        return Err(format!("chunk {} failed after retries: {err}", chunk.index));
                    }

                    attempt += 1;
                    let wait_ms = retry_backoff_ms * u64::from(attempt);
                    let _ = sender.send(ParallelChunkEvent::Retrying {
                        attempt,
                        max_attempts: max_retry_attempts,
                        wait_ms,
                        message: format!("chunk {} retry: {err}", chunk.index),
                    });
                    thread::sleep(Duration::from_millis(wait_ms));
                    break;
                }
            }
        }
    }

    let _ = sender.send(ParallelChunkEvent::Completed);
    Ok(())
}

fn build_direct_request(
    client: &Client,
    request: &DownloadRequest,
) -> reqwest::blocking::RequestBuilder {
    build_direct_request_for_source(client, request, &request.source)
}

fn build_direct_request_for_source(
    client: &Client,
    request: &DownloadRequest,
    source: &str,
) -> reqwest::blocking::RequestBuilder {
    let mut builder = client.get(source);
    builder = builder.header(ACCEPT, HeaderValue::from_static("*/*"));
    if let Some(referrer) = &request.referrer {
        builder = builder.header(REFERER, referrer);
    }
    if let Some(origin) = &request.origin {
        builder = builder.header(ORIGIN, origin);
    }
    if let Some(user_agent) = &request.user_agent {
        builder = builder.header(USER_AGENT, user_agent);
    }
    if let Some(cookie_header) = &request.cookie_header {
        builder = builder.header(COOKIE, cookie_header);
    }
    builder
}

fn requires_browser_context(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    lower.contains("googlevideo.com") || lower.contains("videoplayback")
}

fn probe_content_length(
    client: &Client,
    request: &DownloadRequest,
    source: &str,
) -> Result<Option<u64>, String> {
    let response = build_direct_request_for_source(client, request, source)
        .header(RANGE, "bytes=0-0")
        .send()
        .map_err(|err| format!("request failed: {err}"))?
        .error_for_status()
        .map_err(|err| format!("bad response: {err}"))?;

    Ok(parse_total_bytes(response.headers(), 0))
}

fn download_stream_to_file(
    client: &Client,
    request: &DownloadRequest,
    source: &str,
    target_path: &Path,
    offset_bytes: u64,
    total_bytes: Option<u64>,
    sender: &mpsc::Sender<DirectDownloadEvent>,
    command_receiver: &Receiver<DirectDownloadCommand>,
    started_at: Instant,
) -> Result<u64, String> {
    let mut response = build_direct_request_for_source(client, request, source)
        .send()
        .map_err(|err| format!("request failed: {err}"))?
        .error_for_status()
        .map_err(|err| format!("bad response: {err}"))?;

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(target_path)
        .map_err(|err| format!("open temp file failed: {err}"))?;

    let mut downloaded_bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        match command_receiver.try_recv() {
            Ok(DirectDownloadCommand::Pause) => {
                file.flush().map_err(|err| format!("flush failed: {err}"))?;
                let _ = sender.send(DirectDownloadEvent::Paused {
                    downloaded_bytes: offset_bytes + downloaded_bytes,
                    total_bytes,
                });
                return Err("adaptive download paused".to_owned());
            }
            Err(TryRecvError::Disconnected) => {
                return Err("download control disconnected".to_owned());
            }
            Err(TryRecvError::Empty) => {}
        }

        let read = response
            .read(&mut buffer)
            .map_err(|err| format!("read failed: {err}"))?;
        if read == 0 {
            break;
        }

        file.write_all(&buffer[..read])
            .map_err(|err| format!("write failed: {err}"))?;
        downloaded_bytes += read as u64;
        let aggregate = offset_bytes + downloaded_bytes;
        let elapsed = started_at.elapsed().as_secs_f64().max(0.1);
        let _ = sender.send(DirectDownloadEvent::Progress {
            downloaded_bytes: aggregate,
            total_bytes,
            bytes_per_second: aggregate as f64 / elapsed,
        });
    }

    file.flush().map_err(|err| format!("flush failed: {err}"))?;
    Ok(downloaded_bytes)
}

fn merge_adaptive_streams(video_path: &Path, audio_path: &Path, final_path: &Path) -> Result<(), String> {
    let ffmpeg_path = resolve_ffmpeg_path()
        .ok_or_else(|| bundled_ffmpeg_missing_message())?;
    let mut command = Command::new(&ffmpeg_path);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .args([
            "-y",
            "-i",
            &video_path.display().to_string(),
            "-i",
            &audio_path.display().to_string(),
            "-c",
            "copy",
            &final_path.display().to_string(),
        ]);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let status = command
        .status()
        .map_err(|err| format!("ffmpeg launch failed at {}: {err}", ffmpeg_path.display()))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("ffmpeg merge failed with status {status}"))
    }
}

fn resolve_ffmpeg_path() -> Option<PathBuf> {
    let current_exe = std::env::current_exe().ok()?;
    let exe_dir = current_exe.parent()?;
    let candidates = [
        exe_dir.join("ffmpeg.exe"),
        exe_dir.join("tools").join("ffmpeg.exe"),
        exe_dir.join("tools").join("ffmpeg").join("ffmpeg.exe"),
        exe_dir.join("bin").join("ffmpeg.exe"),
        exe_dir.join("ffmpeg").join("ffmpeg.exe"),
    ];

    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    Some(PathBuf::from("ffmpeg")).filter(|path| Command::new(path).arg("-version").output().is_ok())
}

fn resolve_yt_dlp_path() -> Option<PathBuf> {
    let current_exe = std::env::current_exe().ok()?;
    let exe_dir = current_exe.parent()?;
    let candidates = [
        exe_dir.join("yt-dlp.exe"),
        exe_dir.join("tools").join("yt-dlp.exe"),
        exe_dir.join("tools").join("yt-dlp").join("yt-dlp.exe"),
    ];

    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    Some(PathBuf::from("yt-dlp")).filter(|path| Command::new(path).arg("--version").output().is_ok())
}

fn bundled_ffmpeg_missing_message() -> String {
    "A bundled ffmpeg binary was not found. Place ffmpeg.exe next to NebulaDM.exe or in tools\\ffmpeg.exe for adaptive browser video merging.".to_owned()
}

fn should_use_parallel_download(
    session: &DirectDownloadSession,
    existing_bytes: u64,
    metadata: &ResumeMetadata,
    plan: &DirectDownloadPlan,
) -> bool {
    existing_bytes == 0
        && session.resumed == false
        && plan.parallel_connections > 1
        && metadata.total_bytes.unwrap_or(0) >= plan.chunk_size_bytes * 2
        && metadata.chunks.len() > 1
}

fn build_chunk_progress(
    total_bytes: u64,
    chunk_size_bytes: u64,
    parallel_connections: u32,
) -> Vec<ChunkProgress> {
    if total_bytes == 0 {
        return Vec::new();
    }

    let desired_chunks = ((total_bytes + chunk_size_bytes.saturating_sub(1)) / chunk_size_bytes)
        .max(1)
        .min(u64::from(parallel_connections.max(1)));
    let base_chunk_size = total_bytes / desired_chunks;
    let mut remainder = total_bytes % desired_chunks;
    let mut start = 0_u64;
    let mut chunks = Vec::new();

    for index in 0..desired_chunks {
        let mut current_size = base_chunk_size;
        if remainder > 0 {
            current_size += 1;
            remainder -= 1;
        }
        let end = start + current_size - 1;
        chunks.push(ChunkProgress {
            index: index as u32,
            start_byte: start,
            end_byte: Some(end),
            downloaded_bytes: 0,
        });
        start = end + 1;
    }

    chunks
}

pub fn save_resume_metadata(path: &Path, metadata: &ResumeMetadata) -> Result<(), String> {
    let serialized =
        serde_json::to_string_pretty(metadata).map_err(|err| format!("serialize failed: {err}"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create dir failed: {err}"))?;
    }
    fs::write(path, serialized).map_err(|err| format!("write failed: {err}"))
}

pub fn load_resume_metadata(path: &Path) -> Option<ResumeMetadata> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn parse_total_bytes(headers: &reqwest::header::HeaderMap, existing_bytes: u64) -> Option<u64> {
    if let Some(content_range) = headers
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
    {
        if let Some((_, total)) = content_range.rsplit_once('/') {
            if total != "*" {
                if let Ok(value) = total.parse::<u64>() {
                    return Some(value);
                }
            }
        }
    }

    headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value + existing_bytes)
}

fn sanitize_file_name(file_name: &str) -> String {
    let mut output = String::with_capacity(file_name.len());

    for ch in file_name.chars() {
        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            output.push('_');
        } else {
            output.push(ch);
        }
    }

    let trimmed = output.trim();
    if trimmed.is_empty() {
        "download.bin".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use shared::{DownloadKind, DownloadRequest};

    use crate::categories::default_download_categories;

    use std::path::PathBuf;

    use super::{
        build_direct_download_plan, create_resume_metadata, load_resume_metadata,
        parse_total_bytes, save_resume_metadata,
    };
    use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, HeaderMap, HeaderValue};

    #[test]
    fn direct_download_plan_uses_category_folder() {
        let categories = default_download_categories();
        let request = DownloadRequest::new(
            "setup.exe".to_owned(),
            "https://example.com/setup.exe".to_owned(),
            DownloadKind::Direct,
        );

        let plan = build_direct_download_plan(&request, "Downloads", &categories);

        assert_eq!(plan.target.category_name, "Programs");
        assert_eq!(plan.final_file_path, "Downloads/Programs/setup.exe");
        assert_eq!(plan.temp_file_path, "Downloads/Programs/setup.exe.part");
    }

    #[test]
    fn resume_metadata_starts_with_single_chunk() {
        let categories = default_download_categories();
        let request = DownloadRequest::new(
            "video.mp4".to_owned(),
            "https://example.com/video.mp4".to_owned(),
            DownloadKind::Direct,
        );

        let plan = build_direct_download_plan(&request, "Downloads", &categories);
        let metadata = create_resume_metadata(&request, &plan, Some(1024));

        assert_eq!(metadata.final_file_path, "Downloads/Videos/video.mp4");
        assert_eq!(metadata.chunks.len(), 1);
        assert_eq!(metadata.chunks[0].end_byte, Some(1023));
    }

    #[test]
    fn resume_metadata_can_be_serialized_to_disk() {
        let categories = default_download_categories();
        let request = DownloadRequest::new(
            "archive.zip".to_owned(),
            "https://example.com/archive.zip".to_owned(),
            DownloadKind::Direct,
        );

        let plan = build_direct_download_plan(&request, "Downloads", &categories);
        let metadata = create_resume_metadata(&request, &plan, Some(2048));
        let path = PathBuf::from("target").join("test-resume-metadata.json");

        save_resume_metadata(&path, &metadata).expect("metadata should save");
        assert!(path.exists());
        let loaded = load_resume_metadata(&path).expect("metadata should load");
        assert_eq!(loaded.downloaded_bytes, 0);
        std::fs::remove_file(path).expect("test file should be removable");
    }

    #[test]
    fn direct_download_plan_sets_retry_defaults() {
        let categories = default_download_categories();
        let request = DownloadRequest::new(
            "retry.iso".to_owned(),
            "https://example.com/retry.iso".to_owned(),
            DownloadKind::Direct,
        );

        let plan = build_direct_download_plan(&request, "Downloads", &categories);

        assert_eq!(plan.max_retry_attempts, 4);
        assert_eq!(plan.retry_backoff_ms, 1200);
    }

    #[test]
    fn parse_total_bytes_prefers_content_range_total() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_RANGE, HeaderValue::from_static("bytes 100-199/500"));
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("100"));

        let total = parse_total_bytes(&headers, 100);

        assert_eq!(total, Some(500));
    }

    #[test]
    fn create_resume_metadata_splits_large_downloads_into_parallel_chunks() {
        let categories = default_download_categories();
        let request = DownloadRequest::new(
            "movie.mkv".to_owned(),
            "https://example.com/movie.mkv".to_owned(),
            DownloadKind::Direct,
        );

        let plan = build_direct_download_plan(&request, "Downloads", &categories);
        let metadata = create_resume_metadata(&request, &plan, Some(32 * 1024 * 1024));

        assert!(metadata.chunks.len() > 1);
        assert_eq!(
            metadata.chunks.first().map(|chunk| chunk.start_byte),
            Some(0)
        );
        assert_eq!(
            metadata.chunks.last().and_then(|chunk| chunk.end_byte),
            Some((32 * 1024 * 1024) - 1)
        );
    }
}
