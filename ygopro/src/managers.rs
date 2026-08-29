pub mod data_manager {
    use std::ffi::c_char;
    use std::ffi::c_int;
    use std::ffi::CStr;
    use std::fs;
    use std::sync::Arc;
    use std::sync::LazyLock;

    use arc_swap::ArcSwap;
    use hashbrown::HashMap;
    use parking_lot::Mutex;

    use ygopro_data::constants::Type;
    use ygopro_data::data::Card;
    use ygopro_data::data::CoreCard;

    static SCRIPT_BUFFER: Mutex<[u8; 0x100000]> = Mutex::new([0u8; 0x100000]);
    static GLOBAL_DATA_MANAGER: LazyLock<ArcSwap<DataManager>> = LazyLock::new(|| ArcSwap::from_pointee(DataManager::new()));

    pub fn set_global(data_manager: DataManager) {
        GLOBAL_DATA_MANAGER.store(Arc::new(data_manager));
    }

    pub fn load() -> arc_swap::Guard<Arc<DataManager>> {
        GLOBAL_DATA_MANAGER.load()
    }

    pub fn load_full() -> Arc<DataManager> {
        GLOBAL_DATA_MANAGER.load_full()
    }

    pub fn init() {
        let mut data_manager = DataManager::new();
        #[cfg(feature = "card")]
        {
            let config_manager = super::config_manager::load();
            let db_path = config_manager.get("db_path").unwrap_or("cards.cdb, expansions/*.cdb").to_string();

            for db_pattern in super::config_manager::split_paths(&db_path) {
                let Ok(entries) = glob::glob(db_pattern) else {
                    log::warn!("Failed to parse glob {}", db_pattern);
                    continue;
                };
                for entry in entries {
                    let path = entry.map_err(|err| log::warn!("Failed to read glob entry {}: {:?}", db_pattern, err)).ok();
                    if let Some(path) = path {
                        #[cfg(feature = "zip")]
                        if let Some(bytes) = crate::ypk::archive_manager::read_file(&path.to_string_lossy()) {
                            data_manager.load_db_from_bytes(&bytes)
                                .map(|()| log::trace!("Loaded database {}", path.display()))
                                .map_err(|err| log::warn!("Failed to load database {}: {:?}", path.display(), err)).ok();
                        }
                        #[cfg(not(feature = "zip"))]
                        data_manager.load_db(&path.to_string_lossy())
                            .map(|()| log::trace!("Loaded database {}", path.display()))
                            .map_err(|err| log::warn!("Failed to load database {}: {:?}", path.display(), err)).ok();
                    }
                }
            }
        }
        #[cfg(all(feature = "card", feature = "zip"))]
        for cdb_name in crate::ypk::archive_manager::cdb_names() {
            if let Some(bytes) = crate::ypk::archive_manager::read_file(&cdb_name) {
                data_manager.load_db_from_bytes(&bytes)
                    .map(|()| log::trace!("Loaded database {}", cdb_name))
                    .map_err(|err| log::warn!("Failed to load database {}: {:?}", cdb_name, err)).ok();
            }
        }
        data_manager.finalize_db();
        set_global(data_manager);
    }

    #[cfg(feature = "card")]
    pub const CARD_ARTWORK_VERSIONS_OFFSET: u32 = 20;

    #[cfg(feature = "card")]
    fn is_alternative(code: u32, alias: u32) -> bool {
        alias != 0 && alias < code + CARD_ARTWORK_VERSIONS_OFFSET && code < alias + CARD_ARTWORK_VERSIONS_OFFSET
    }

    pub struct DataManager {
        pub cards: HashMap<u32, Card>,
        pub extra_setcode: HashMap<u32, Vec<u16>>,
    }

    impl DataManager {
        pub fn new() -> Self {
            let mut extra_setcode = HashMap::new();
            extra_setcode.insert(8512558u32, vec![0x8f, 0x54, 0x59, 0x82, 0x13a]);
            extra_setcode.insert(55088578u32, vec![0x8f, 0x54, 0x59, 0x82, 0x13a]);
            Self {
                cards: HashMap::new(),
                extra_setcode,
            }
        }

        #[cfg(feature = "card")]
        pub fn load_db(&mut self, file: &str) -> Result<(), String> {
            let cards = ygopro_data::data::load_db_from_file::<Card>(file)
                .map_err(|e| format!("Failed to load {}: {}", file, e))?;
            self.insert_cards(cards);
            log::trace!("Loaded database {}", file);
            Ok(())
        }

        #[cfg(feature = "card")]
        pub fn load_db_from_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
            let cards = ygopro_data::data::load_db_from_bytes::<Card>(bytes)
                .map_err(|e| format!("Failed to load database: {}", e))?;
            self.insert_cards(cards);
            Ok(())
        }

        #[cfg(feature = "card")]
        fn insert_cards(&mut self, cards: Vec<Card>) {
            for mut card in cards {
                if card.code == 5405695 {
                    card.rule_code = card.alias;
                    card.alias = 0;
                } else if card.alias != 0
                    && !card.card_type.contains(Type::Token)
                    && !is_alternative(card.code, card.alias)
                {
                    card.rule_code = card.alias;
                    card.alias = 0;
                }
                self.cards.insert(card.code, card);
            }
        }

        pub fn finalize_db(&mut self) {
            let pending: Vec<(u32, u32)> = self
                .cards
                .iter()
                .filter_map(|(&code, card)| {
                    if card.rule_code != 0 || card.alias == 0 || card.card_type.contains(Type::Token) {
                        return None;
                    }
                    Some((code, card.alias))
                })
                .collect();

            for (code, alias) in pending {
                let rule_code = self.cards.get(&alias).map(|c| c.rule_code).unwrap_or(0);
                if let Some(card) = self.cards.get_mut(&code) {
                    card.rule_code = rule_code;
                }
            }

            for (code, list) in &self.extra_setcode {
                if list.is_empty() || list.len() > 16 { continue; }
                if let Some(card) = self.cards.get_mut(code) {
                    for (i, &sc) in list.iter().enumerate() {
                        card.setcode[i] = sc;
                    }
                }
            }
        }

        pub fn get_card(&self, code: u32) -> Option<&Card> {
            self.cards.get(&code)
        }

        pub fn get_core_card(&self, code: u32) -> Option<&CoreCard> {
            self.cards.get(&code).map(|c| &c.card)
        }
    }

    pub extern "C" fn card_reader(code: u32, data: *mut CoreCard) -> u32 {
        if data.is_null() {
            return 0;
        }
        let guard = GLOBAL_DATA_MANAGER.load();
        if let Some(card) = guard.get_core_card(code) {
            unsafe { *data = card.clone(); }
            return 0;
        }
        unsafe { *data = CoreCard::default(); }
        0
    }

    /// Corresponds to `ScriptReaderEx` in data_manager.cpp:493-517.
    pub extern "C" fn script_reader(script_path: *const c_char, slen: *mut c_int) -> *mut u8 {
        fn read_file(file_path: &str, buffer: &mut [u8]) -> Option<usize> {
            fs::read(file_path).ok().and_then(|data| {
                if data.len() >= buffer.len() {
                    return None;
                }
                buffer[..data.len()].copy_from_slice(&data);
                Some(data.len())
            })
        }
        #[cfg(feature = "zip")]
        fn read_archive(archive_path: &str, buffer: &mut [u8]) -> Option<usize> {
            crate::ypk::archive_manager::read_from_archives(archive_path).and_then(|data| {
                if data.len() >= buffer.len() {
                    return None;
                }
                buffer[..data.len()].copy_from_slice(&data);
                Some(data.len())
            })
        }

        if script_path.is_null() || slen.is_null() {
            return std::ptr::null_mut();
        }
        let path = unsafe { CStr::from_ptr(script_path).to_string_lossy() };
        let mut buffer = SCRIPT_BUFFER.lock();

        if !path.starts_with("./script") {
            if let Some(len) = read_file(path.as_ref(), &mut *buffer) {
                unsafe { *slen = len as c_int; }
                return buffer.as_mut_ptr();
            }
            return std::ptr::null_mut();
        }

        let script_name = &path[2..];
        let expansions_path = format!("./expansions/{}", script_name);
        let config_manager = super::config_manager::load();
        let prefer_expansion_script = config_manager.get("prefer_expansion_script").map(|value| value.trim() != "0").unwrap_or(false);

        if prefer_expansion_script {
            if let Some(len) = read_file(&expansions_path, &mut *buffer) {
                unsafe { *slen = len as c_int; }
                return buffer.as_mut_ptr();
            }
            #[cfg(feature = "zip")]
            if let Some(len) = read_archive(script_name, &mut *buffer) {
                unsafe { *slen = len as c_int; }
                return buffer.as_mut_ptr();
            }
            if let Some(len) = read_file(path.as_ref(), &mut *buffer) {
                unsafe { *slen = len as c_int; }
                return buffer.as_mut_ptr();
            }
        } else {
            #[cfg(feature = "zip")]
            if let Some(len) = read_archive(script_name, &mut *buffer) {
                unsafe { *slen = len as c_int; }
                return buffer.as_mut_ptr();
            }
            if let Some(len) = read_file(path.as_ref(), &mut *buffer) {
                unsafe { *slen = len as c_int; }
                return buffer.as_mut_ptr();
            }
            if let Some(len) = read_file(&expansions_path, &mut *buffer) {
                unsafe { *slen = len as c_int; }
                return buffer.as_mut_ptr();
            }
        }

        std::ptr::null_mut()
    }
}

pub mod i18n {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::LazyLock;

    use arc_swap::ArcSwap;

    static GLOBAL_STRINGS: LazyLock<ArcSwap<HashMap<String, HashMap<i32, String>>>> = LazyLock::new(|| ArcSwap::from_pointee(HashMap::new()));

    pub fn set_strings(strings: HashMap<String, HashMap<i32, String>>) {
        GLOBAL_STRINGS.store(Arc::new(strings));
    }

    pub fn init() {
        let strings = ygopro_data::data::load_strings_conf("strings.conf");
        set_strings(strings);
    }
}

pub mod deck_manager {
    use std::collections::HashMap;
    use std::fs;
    use std::io;
    use std::sync::Arc;
    use std::sync::LazyLock;

    use arc_swap::ArcSwap;

    use ygopro_data::data::parse_lflist_content;
    use ygopro_data::data::LFList;

    static GLOBAL_DECK_MANAGER: LazyLock<ArcSwap<DeckManager>> = LazyLock::new(|| ArcSwap::from_pointee(DeckManager::new()));

    pub fn set_global(deck_manager: DeckManager) {
        GLOBAL_DECK_MANAGER.store(Arc::new(deck_manager));
    }

    pub fn load() -> arc_swap::Guard<Arc<DeckManager>> {
        GLOBAL_DECK_MANAGER.load()
    }

    pub fn init() {
        let config_manager = super::config_manager::load();
        let lflist_path = config_manager.get("lflist_path").unwrap_or("expansions/lflist.conf, lflist.conf").to_string();

        let mut deck_manager = DeckManager::new();
        for lflist_pattern in super::config_manager::split_paths(&lflist_path) {
            let Ok(entries) = glob::glob(lflist_pattern) else {
                log::warn!("Failed to parse glob {}", lflist_pattern);
                continue;
            };
            for entry in entries {
                let path = entry.map_err(|err| log::warn!("Failed to read glob entry {}: {:?}", lflist_pattern, err)).ok();
                if let Some(path) = path {
                    deck_manager.load_lflist(&path.to_string_lossy())
                        .map_err(|err| log::warn!("Failed to read lflist {}: {:?}", path.display(), err)).ok();
                }
            }
        }
        if !deck_manager.lflists.is_empty() {
            deck_manager.lflists.push(LFList {
                hash: 0,
                name: "N/A".to_string(),
                content: HashMap::new(),
                genesys: 0,
                glist: HashMap::new(),
            });
        }
        set_global(deck_manager);
    }

    pub struct DeckManager {
        pub lflists: Vec<LFList>,
    }

    impl DeckManager {
        pub fn new() -> Self {
            Self {
                lflists: Vec::new(),
            }
        }

        pub fn load_lflist(&mut self, path: &str) -> io::Result<()> {
            let content = fs::read_to_string(path)?;
            self.lflists.extend(parse_lflist_content(&content));
            log::trace!("Loaded lflist {}", path);
            Ok(())
        }

        pub fn get_lflist_by_index(&self, index: u32) -> Option<&LFList> {
            self.lflists.get(index as usize)
        }

        pub fn get_lflist_by_hash(&self, hash: u32) -> Option<&LFList> {
            self.lflists.iter().find(|l| l.hash == hash)
        }

        pub fn get_lflist_name(&self, hash: u32) -> &str {
            self.get_lflist_by_hash(hash)
                .map(|l| l.name.as_str())
                .unwrap_or("???")
        }
    }
}

pub mod config_manager {
    use std::fs;
    use std::io;
    use std::sync::Arc;
    use std::sync::LazyLock;

    use arc_swap::ArcSwap;
    use hashbrown::HashMap;

    static GLOBAL_CONFIG_MANAGER: LazyLock<ArcSwap<ConfigManager>> = LazyLock::new(|| ArcSwap::from_pointee(ConfigManager::new()));

    pub fn set_global(config_manager: ConfigManager) {
        GLOBAL_CONFIG_MANAGER.store(Arc::new(config_manager));
    }

    pub fn load() -> arc_swap::Guard<Arc<ConfigManager>> {
        GLOBAL_CONFIG_MANAGER.load()
    }

    pub fn init() {
        let config_path = std::env::var("YGOPRO_CONFIG_PATH").unwrap_or_else(|_| "system.conf".to_string());

        let mut config_manager = ConfigManager::new();
        config_manager.load(&config_path).ok();
        config_manager.load_env();
        set_global(config_manager);
    }

    pub fn split_paths(path: &str) -> Vec<&str> {
        path.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect()
    }

    pub struct ConfigManager {
        entries: HashMap<String, String>,
    }

    impl ConfigManager {
        pub fn new() -> Self {
            Self {
                entries: HashMap::new(),
            }
        }

        pub fn load(&mut self, path: &str) -> io::Result<()> {
            let content = fs::read_to_string(path)?;
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some(eq) = line.find('=') {
                    let key = line[..eq].trim().to_string();
                    let value = line[eq + 1..].trim().to_string();
                    self.entries.insert(key, value);
                }
            }
            Ok(())
        }

        pub fn load_env(&mut self) {
            for (key, value) in std::env::vars() {
                if let Some(name) = key.strip_prefix("YGOPRO_") {
                    self.entries.insert(name.to_lowercase(), value);
                }
            }
        }

        pub fn get(&self, key: &str) -> Option<&str> {
            self.entries.get(key).map(|s| s.as_str())
        }

        pub fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
            self.entries
                .get(key)
                .map(|s| s.as_str())
                .unwrap_or(default)
        }
    }
}

pub use data_manager::DataManager;
pub use deck_manager::DeckManager;
pub use config_manager::ConfigManager;
pub use i18n::set_strings;
