use std::collections::HashMap;

const HASH_INITIAL_VALUE: u32 = 0x7dfcee6a;
const GENESYS_HASH_MARKER: u32 = 0x43524544;
const GENESYS_HASH_KEY: u32 = 0x965f7da9;

#[derive(Debug, Clone)]
pub struct LFList {
    pub hash: u32,
    pub name: String,
    pub content: HashMap<u32, u8>,
    pub genesys: u32,
    pub glist: HashMap<u32, u32>,
}

impl LFList {
    pub fn new(name: String) -> Self {
        Self {
            hash: HASH_INITIAL_VALUE,
            name,
            genesys: 0,
            content: HashMap::new(),
            glist: HashMap::new(),
        }
    }

    pub fn from(name: String, content: HashMap<u32, u8>, genesys: u32, glist: HashMap<u32, u32>) -> Self {
        let mut v = Self {
            hash: HASH_INITIAL_VALUE,
            name,
            content,
            genesys,
            glist,
        };
        v.calculate_hash();
        v
    }

    pub fn calculate_hash(&mut self) {
        let mut hash = HASH_INITIAL_VALUE;
        if self.genesys > 0 {
            hash ^= ((GENESYS_HASH_KEY  << 18) | (GENESYS_HASH_KEY  >> 14)) ^ ((self.genesys << 9) | (self.genesys >> 23)) ^ ((GENESYS_HASH_MARKER << 27) | (GENESYS_HASH_MARKER >> 5));
        }
        for (&code, &ct) in &self.content {
            hash ^= ((code << 18) | (code >> 14)) ^ ((code << (27 + ct)) | (code >> (5 - ct)));
        }
        for (&code, &ct) in &self.glist {
            hash ^= ((code << 18) | (code >> 14)) ^ ((GENESYS_HASH_KEY  << 9) | (GENESYS_HASH_KEY  >> 23)) ^ ((ct << 27) | (ct >> 5));
        }
        self.hash = hash;
    }
    
}

pub fn parse_lflist_content(content: &str) -> Vec<LFList> {
    let mut lists = Vec::new();
    let mut name = String::new();
    let mut limits = HashMap::new();
    let mut genesys = 0;
    let mut genesys_limits = HashMap::new();
    for line in content.lines().map(|l| l.trim()) {
        if line.is_empty() || line.starts_with('#') { continue; }
        if line.starts_with('!') {
            if !name.is_empty() {
                lists.push(LFList::from(
                    std::mem::take(&mut name),
                    std::mem::take(&mut limits),
                    std::mem::take(&mut genesys),
                    std::mem::take(&mut genesys_limits),
                ));
            }
            name = line[1..].to_string();
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            if parts[0] == "$genesys" {
                genesys = parts[1].trim().parse::<u32>().unwrap_or(0);
            } else if let (Ok(card_code), Ok(limit)) = (u32::from_str_radix(parts[0].trim(), 10), parts[1].trim().parse::<u8>()) {
                if limit <= 2 { limits.insert(card_code, limit); }
            } else if parts.len() >= 3 && parts[1] == "$genesys" {
                if let (Ok(card_code), Ok(limit)) = (u32::from_str_radix(parts[0].trim(), 10), parts[2].trim().parse::<u32>()) {
                    genesys_limits.insert(card_code, limit);
                }
            }
        }
    }
    if !name.is_empty() { lists.push(LFList::from(name, limits, genesys, genesys_limits)); }
    lists
}

#[cfg(test)]
mod tests {
    use crate::data::lflist::GENESYS_HASH_KEY;
    use crate::data::lflist::parse_lflist_content;

    /// How we get genesys hash key.
    #[test]
    #[ignore]
    fn genesys_genesys_hash_key() {
        let mut hash: u32 = 2166136261;
        for byte in "genesys".bytes() {
            hash ^= byte as u32;
            hash = hash.wrapping_mul(16777619);
        }
        println!("Computed genesys hash key: 0x{:08x}", hash);
        assert_eq!(GENESYS_HASH_KEY, hash);
    }

    /// Reference C++ code used to compute the expected hash:
    ///
    /// ```
    /// #include <cstdint>
    /// #include <cstdio>
    /// #include <unordered_map>
    ///
    /// int main() {
    ///     const uint32_t HASH_INITIAL_VALUE = 0x7dfcee6a;
    ///     std::unordered_map<uint32_t, uint8_t> content = {
    ///         {20292186, 0},
    ///         {91869203, 0},
    ///         {8633261, 1},
    ///         {79606837, 1},
    ///         {23434538, 1},
    ///         {72270339, 2},
    ///     };
    ///     uint32_t hash = HASH_INITIAL_VALUE;
    ///     for (auto& [code, count] : content) {
    ///         hash = hash ^ ((code << 18) | (code >> 14)) ^ ((code << (27 + count)) | (code >> (5 - count)));
    ///     }
    ///     std::printf("hash = 0x%08x\n", hash);
    /// }
    /// ```
    #[test]
    fn calculate_hash_matches_cpp_reference() {
        let content = "
!test
20292186 0
91869203 0
8633261 1
79606837 1
23434538 1
72270339 2
";
        let lists = parse_lflist_content(content);
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].hash, 0xdee2c97b);
    }
}
