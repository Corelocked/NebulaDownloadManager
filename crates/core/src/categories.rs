use shared::DownloadCategory;

pub fn default_download_categories() -> Vec<DownloadCategory> {
    vec![
        DownloadCategory {
            name: "Videos".to_owned(),
            folder_name: "Videos".to_owned(),
            extensions: vec!["mp4", "mkv", "avi", "mov"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        },
        DownloadCategory {
            name: "Music".to_owned(),
            folder_name: "Music".to_owned(),
            extensions: vec!["mp3", "wav", "flac", "aac"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        },
        DownloadCategory {
            name: "Documents".to_owned(),
            folder_name: "Documents".to_owned(),
            extensions: vec!["pdf", "docx", "xlsx", "pptx", "txt"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        },
        DownloadCategory {
            name: "Programs".to_owned(),
            folder_name: "Programs".to_owned(),
            extensions: vec!["exe", "msi", "zip", "rar", "7z"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        },
        DownloadCategory {
            name: "Torrents".to_owned(),
            folder_name: "Torrents".to_owned(),
            extensions: vec!["torrent"].into_iter().map(str::to_owned).collect(),
        },
        DownloadCategory {
            name: "Other".to_owned(),
            folder_name: "Other".to_owned(),
            extensions: Vec::new(),
        },
    ]
}
