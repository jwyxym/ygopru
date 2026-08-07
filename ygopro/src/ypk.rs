pub mod archive_manager {
    use std::collections::HashSet;
    use std::fs;
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use arc_swap::ArcSwapOption;
    use parking_lot::Mutex;
    use zip::ZipArchive;

    struct ExpansionArchive {
        _path: PathBuf,
        archive_reader: Mutex<ZipArchive<fs::File>>,
    }

    static GLOBAL_ARCHIVES: ArcSwapOption<Vec<ExpansionArchive>> = ArcSwapOption::const_empty();

    pub fn init() {
        let path: String = crate::managers::config_manager::load()
            .as_ref()
            .and_then(|config_manager| config_manager.get("path"))
            .unwrap_or("./")
            .to_string();
        let path: &Path = Path::new(&path);
        let expansions_path: PathBuf = path.join("expansions");
        let pack_names: HashSet<String> = crate::managers::config_manager::load()
            .as_ref()
            .and_then(|config_manager| config_manager.get("pack_names"))
            .map(|pack_names| {
                pack_names
                    .split('/')
                    .map(|pack_name| pack_name.trim().to_string())
                    .filter(|pack_name| !pack_name.is_empty())
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let mut expansion_archives: Vec<ExpansionArchive> = Vec::new();
        for pack_name in &pack_names {
            let pack_path: PathBuf = expansions_path.join(pack_name);
            let Ok(file) = fs::File::open(&pack_path) else {
                log::debug!("Failed to open archive {}", pack_name);
                continue;
            };
            match ZipArchive::new(file) {
                Ok(archive_reader) => expansion_archives.push(ExpansionArchive {
                    _path: pack_path,
                    archive_reader: Mutex::new(archive_reader),
                }),
                Err(error) => log::debug!("Failed to open archive {}: {}", pack_name, error),
            }
        }
        GLOBAL_ARCHIVES.store(Some(Arc::new(expansion_archives)));
    }

    pub fn read_from_archives(name: &str) -> Option<Vec<u8>> {
        let guard = GLOBAL_ARCHIVES.load();
        for expansion_archive in (*guard).as_ref()?.iter() {
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

    pub fn read_file(name: &str) -> Option<Vec<u8>> {
        match fs::read(name) {
            Ok(data) => Some(data),
            Err(_) => read_from_archives(name),
        }
    }

    pub fn cdb_names() -> Vec<String> {
        let guard = GLOBAL_ARCHIVES.load();
        let mut names = Vec::new();
        if let Some(archives) = (*guard).as_ref() {
            for expansion_archive in archives.iter() {
                let mut archive_reader = expansion_archive.archive_reader.lock();
                for index in 0..archive_reader.len() {
                    if let Ok(file) = archive_reader.by_index(index) {
                        if file.name().ends_with(".cdb") {
                            names.push(file.name().to_string());
                        }
                    }
                }
            }
        }
        names
    }
}
