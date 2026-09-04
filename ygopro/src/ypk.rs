pub mod archive_manager {
    use std::fs;
    use std::io::Read;
    use std::sync::Arc;
    use std::sync::LazyLock;

    use arc_swap::ArcSwap;
    use parking_lot::Mutex;
    use zip::ZipArchive;

    struct ExpansionArchive {
        _path: String,
        archive_reader: Mutex<ZipArchive<fs::File>>,
    }

    static GLOBAL_ARCHIVES: LazyLock<ArcSwap<Vec<ExpansionArchive>>> = LazyLock::new(|| ArcSwap::from_pointee(Vec::new()));

    pub fn init() {
        let mut expansion_archives = Vec::new();
        let entries = if let Ok(entries) = fs::read_dir("./expansions") {
            entries 
        } else {
            log::debug!("Failed to read directory ./expansions, it may not exists");
            GLOBAL_ARCHIVES.store(Arc::new(expansion_archives));
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !is_expansion_archive(&path) {
                continue;
            }
            let Ok(file) = fs::File::open(&path) else {
                log::debug!("Failed to open archive {}", path.display());
                continue;
            };
            match ZipArchive::new(file) {
                Ok(archive_reader) => expansion_archives.push(ExpansionArchive {
                    _path: path.display().to_string(),
                    archive_reader: Mutex::new(archive_reader),
                }),
                Err(error) => log::debug!("Failed to open archive {}: {}", path.display(), error),
            }
        }
        GLOBAL_ARCHIVES.store(Arc::new(expansion_archives));
    }

    pub fn read_from_archives(name: &str) -> Option<Vec<u8>> {
        let guard = GLOBAL_ARCHIVES.load();
        for expansion_archive in guard.iter() {
            let mut archive_reader = expansion_archive.archive_reader.lock();
            if let Ok(mut file) = archive_reader.by_name(name) {
                let mut buffer = Vec::new();
                if file.read_to_end(&mut buffer).is_ok() {
                    return Some(buffer);
                }
            }
        }
        None
    }

    #[cfg(feature = "card")]
    pub fn read_file(name: &str) -> Option<Vec<u8>> {
        match fs::read(name) {
            Ok(data) => Some(data),
            Err(_) => read_from_archives(name),
        }
    }

    #[cfg(feature = "card")]
    pub fn cdb_names() -> Vec<String> {
        let guard = GLOBAL_ARCHIVES.load();
        let mut names = Vec::new();
        for expansion_archive in guard.iter() {
            let mut archive_reader = expansion_archive.archive_reader.lock();
            for index in 0..archive_reader.len() {
                if let Ok(file) = archive_reader.by_index(index) {
                    if file.name().ends_with(".cdb") {
                        names.push(file.name().to_string());
                    }
                }
            }
        }
        names
    }

    fn is_expansion_archive(path: &std::path::Path) -> bool {
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension == "zip" || extension == "ypk")
            .unwrap_or(false)
    }
}
