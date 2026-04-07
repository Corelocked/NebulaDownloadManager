use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=../../tools/ffmpeg/ffmpeg.exe");
    println!("cargo:rerun-if-changed=../../tools/yt-dlp/yt-dlp.exe");

    let out_dir = match env::var_os("OUT_DIR") {
        Some(value) => PathBuf::from(value),
        None => return,
    };
    let manifest_dir = match env::var_os("CARGO_MANIFEST_DIR") {
        Some(value) => PathBuf::from(value),
        None => return,
    };

    let target_profile_dir = match out_dir.ancestors().nth(3) {
        Some(path) => path.to_path_buf(),
        None => return,
    };
    let tools_dir = target_profile_dir.join("tools");
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf);

    let Some(repo_root) = repo_root else {
        return;
    };

    let _ = fs::create_dir_all(&tools_dir);
    copy_if_present(
        &repo_root.join("tools").join("ffmpeg").join("ffmpeg.exe"),
        &tools_dir.join("ffmpeg.exe"),
    );
    copy_if_present(
        &repo_root.join("tools").join("yt-dlp").join("yt-dlp.exe"),
        &tools_dir.join("yt-dlp.exe"),
    );
}

fn copy_if_present(source: &Path, destination: &Path) {
    if source.is_file() {
        let _ = fs::copy(source, destination);
    }
}
