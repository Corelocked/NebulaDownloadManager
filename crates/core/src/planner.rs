use shared::{DownloadCategory, DownloadKind, DownloadPlan, DownloadRequest};

pub fn plan_download(
    request: &DownloadRequest,
    downloads_root: &str,
    categories: &[DownloadCategory],
) -> DownloadPlan {
    let extension = request
        .file_name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    let category = match request.kind {
        DownloadKind::Torrent => categories
            .iter()
            .find(|category| category.name == "Torrents")
            .or_else(|| categories.iter().find(|category| category.name == "Other")),
        DownloadKind::Direct => categories.iter().find(|category| {
            !category.extensions.is_empty()
                && category
                    .extensions
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(&extension))
        }),
    }
    .or_else(|| categories.iter().find(|category| category.name == "Other"));

    let fallback = DownloadPlan {
        category_name: "Other".to_owned(),
        target_folder: format!("{downloads_root}/Other"),
    };

    match category {
        Some(category) => DownloadPlan {
            category_name: category.name.clone(),
            target_folder: format!("{downloads_root}/{}", category.folder_name),
        },
        None => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::plan_download;
    use crate::categories::default_download_categories;
    use shared::{DownloadKind, DownloadRequest};

    #[test]
    fn routes_programs_to_program_folder() {
        let categories = default_download_categories();
        let request = DownloadRequest::new(
            "installer.exe".to_owned(),
            "https://example.com/installer.exe".to_owned(),
            DownloadKind::Direct,
        );

        let plan = plan_download(&request, "Downloads", &categories);

        assert_eq!(plan.category_name, "Programs");
        assert_eq!(plan.target_folder, "Downloads/Programs");
    }

    #[test]
    fn routes_torrents_to_torrent_folder() {
        let categories = default_download_categories();
        let request = DownloadRequest::new(
            "ubuntu.torrent".to_owned(),
            "magnet:?xt=urn:btih:123".to_owned(),
            DownloadKind::Torrent,
        );

        let plan = plan_download(&request, "Downloads", &categories);

        assert_eq!(plan.category_name, "Torrents");
        assert_eq!(plan.target_folder, "Downloads/Torrents");
    }
}
