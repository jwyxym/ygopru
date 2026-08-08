use std::io::Cursor;
use std::io::Read;
use std::ops::Deref;

use binrw::BinRead;
use binrw::BinResult;
use binrw::BinWrite;
use binrw::binrw;
use binrw::helpers::until_eof;
use bitflags::bitflags;

use lzma_rs::lzma_compress_with_options;
use lzma_rs::lzma_decompress_with_options;

use crate::constants::Mode;
use crate::constants::Rule;
use crate::data::Deck;
use crate::data::Response;
use crate::message::HostInfo;
use crate::utils::string::FixedLengthString;

const SIZE_REPLAY_SEED: usize = 8;

#[repr(u32)]
pub enum ReplayVersion {
    V1 = 0x31707279,
    V2 = 0x32707279
}

bitflags! {
    #[derive(BinRead, BinWrite, Clone, Debug, PartialEq, Eq)]
    #[br(map=|x: u32| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct ReplayHeaderFlags: u32 {
        const Compressed = 1;
        const Tag = 2;
        const Decode = 4;
        const SingleMode = 8;
        const Uniform = 16;
    }
}

bitflags! {
    #[derive(BinRead, BinWrite, Clone, Debug)]
    #[br(map=|x: u16| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct DuelOptions: u16 {
        const TestMode = 0x1;
        const AttackFirstTurn = 0x2;
        const OldReplay = 0x4;
        const ObsoleteRuling = 0x8;
        const PseudoShuffle = 0x10;
        const TagMode = 0x20;
        const SimpleAI = 0x40;
        const ReturnDeckTop = 0x80;
        const RevealDeckSequence = 0x100;
    }
}

bitflags! {
    #[derive(BinRead, BinWrite, Clone, Debug)]
    #[br(map=|x: u32| Self::from_bits_retain(x))]
    #[bw(map=|x: &Self| x.bits())]
    pub struct ReplayMode: u32 {
        const SaveInServer = 1;
        const WatcherNoSend = 2;
        const IncludeChat = 4;
    }
}

#[derive(BinRead, BinWrite, Clone, Debug)]
pub struct ReplayHeader {
    pub id: u32,
    pub version: u32,
    pub flag: ReplayHeaderFlags,
    pub seed: u32,
    pub data_size: u32,
    pub start_time: u32,
    pub props: [u8; 8],
    #[br(if(id == ReplayVersion::V2 as u32))]
    #[bw(if(*id == ReplayVersion::V2 as u32))]
    pub seed_sequence: [u32; SIZE_REPLAY_SEED],
    #[br(if(id == ReplayVersion::V2 as u32))]
    #[bw(if(*id == ReplayVersion::V2 as u32))]
    pub header_version: u32,
    #[br(if(id == ReplayVersion::V2 as u32))]
    #[bw(if(*id == ReplayVersion::V2 as u32))]
    pub reserved: [u32; 3],
}

impl ReplayHeader {
    pub fn is_compressed(&self) -> bool { self.flag.contains(ReplayHeaderFlags::Compressed) }
    pub fn is_tag(&self)        -> bool { self.flag.contains(ReplayHeaderFlags::Tag) }
    pub fn is_decoded(&self)    -> bool { self.flag.contains(ReplayHeaderFlags::Decode) }
    pub fn is_single_mode(&self)-> bool { self.flag.contains(ReplayHeaderFlags::SingleMode) }
    pub fn is_uniform(&self)    -> bool { self.flag.contains(ReplayHeaderFlags::Uniform) }
}

#[binrw]
#[derive(PartialEq, Eq, Debug, Clone, Default)]
pub struct ReplayDeck {
    #[bw(calc = main.len() as u32)]
    main_size: u32,
    #[br(count = main_size)]
    pub main: Vec<u32>,
    #[bw(calc = extra.len() as u32)]
    extra_size: u32,
    #[br(count = extra_size)]
    pub extra: Vec<u32>,
}

impl From<Deck> for ReplayDeck {
    fn from(value: Deck) -> Self {
        let mut main = value.main;
        let mut extra = value.extra;
        main.reverse();
        extra.reverse();
        Self { main, extra }
    }
}

impl From<ReplayDeck> for Deck {
    fn from(value: ReplayDeck) -> Self {
        let mut main = value.main.clone();
        let mut extra = value.extra.clone();
        main.reverse();
        extra.reverse();
        Self { main, side: vec![], extra }
    }
}

#[binrw]
#[derive(Clone, Debug)]
#[br(import(header: &ReplayHeader))]
#[bw(import(header: &ReplayHeader))]
pub struct ReplayBody {
    pub host_name: FixedLengthString<20>,
    pub client_name: FixedLengthString<20>,
    #[br(if(header.is_tag()))]
    #[bw(if(header.is_tag()))]
    pub tag_host_name: Option<FixedLengthString<20>>,
    #[br(if(header.is_tag()))]
    #[bw(if(header.is_tag()))]
    pub tag_client_name: Option<FixedLengthString<20>>,
    pub start_lp: u32,
    pub start_hand: u32,
    pub draw_count: u32,
    // pub opt: u32, -> Split into two parts...
    pub duel_options: DuelOptions,
    pub duel_rule: u16,
    pub host_deck: ReplayDeck,
    pub client_deck: ReplayDeck,
    #[br(if(header.is_tag()))]
    #[bw(if(header.is_tag()))]
    pub tag_host_deck: Option<ReplayDeck>,
    #[br(if(header.is_tag()))]
    #[bw(if(header.is_tag()))]
    pub tag_client_deck: Option<ReplayDeck>,
    #[br(parse_with=until_eof)]
    pub datas: Vec<ReplayData>
}

#[binrw]
#[derive(Clone, Debug)]
pub struct ReplayData {
    #[bw(calc(data.len() as u8))]
    size: u8,
    #[br(count = size, map = |bytes: Vec<u8>| Response::Unknown(bytes))]
    pub data: Response
}

#[derive(BinRead, BinWrite, Debug, Clone)]
pub struct Replay {
    pub header: ReplayHeader,
    #[br(parse_with = replay_parser, args(&header))]
    #[bw(write_with = replay_writer, args(&header))]
    pub body: ReplayBody
}

impl Replay {
    pub fn fill_data_size(&mut self) {
        let host = &self.body.host_deck;
        let client = &self.body.client_deck;
        let mut size = 40 + 40
            + 4 + 4 + 4 + 2 + 2
            + 4 + host.main.len() as u32 * 4 + 4 + host.extra.len() as u32 * 4
            + 4 + client.main.len() as u32 * 4 + 4 + client.extra.len() as u32 * 4
            + self.body.datas.iter().map(|d| 1 + d.data.len() as u32).sum::<u32>();
            
        if self.header.is_tag() {
            size += 40 + 40;
            if let Some(ref tag_host) = self.body.tag_host_deck {
                size += 4 + tag_host.main.len() as u32 * 4 + 4 + tag_host.extra.len() as u32 * 4;
            }
            if let Some(ref tag_client) = self.body.tag_client_deck {
                size += 4 + tag_client.main.len() as u32 * 4 + 4 + tag_client.extra.len() as u32 * 4;
            }
        }
        self.header.data_size = size;
    }

    pub fn duel_rule(&self) -> crate::constants::MasterRule { 
        if self.duel_options.contains(DuelOptions::ObsoleteRuling) { crate::constants::MasterRule::MasterRule1 }
        else { crate::constants::MasterRule::try_from(self.duel_rule as u8).unwrap_or(crate::constants::MasterRule::MasterRule1) }
    }

    pub fn mode(&self) -> Mode {
        if self.duel_options.contains(DuelOptions::TagMode) { Mode::Tag }
        else { Mode::Single }
    } 
    pub fn no_shuffle_deck(&self) -> bool { self.duel_options.contains(DuelOptions::PseudoShuffle) }
    pub fn is_tag(&self) -> bool { self.duel_options.contains(DuelOptions::TagMode) }

    pub fn host_info(&self) -> HostInfo {
        HostInfo { 
            lflist: 999,
            rule: Rule::OCG,
            mode: self.mode(),
            duel_rule: self.duel_rule(), 
            no_check_deck: true,
            no_shuffle_deck: self.no_shuffle_deck(), 
            start_lp: self.start_lp, 
            start_hand: self.start_hand as u8, 
            draw_count: self.draw_count as u8, 
            time_limit: 999 
        }
    }
}

impl Deref for Replay {
    type Target = ReplayBody;

    fn deref(&self) -> &Self::Target {
        &self.body
    }
}

#[derive(BinRead)]
struct ReadHelper {
    #[br(parse_with = until_eof)]
    content: Vec<u8>
}

// ==================================================
// Correct order: 
// prop  dict_size  datasize
//  93    0 0 0 1     u64
// Ygopro replay header:
// datasize  prop  dict_size  padding
//   u32      93    0 0 0 1    0 0 0
// ==================================================
#[binrw::parser(reader, endian)]
fn replay_parser(header: &ReplayHeader) -> BinResult<ReplayBody> {
    let leading_props = Cursor::new(&header.props[0..5]);
    let helper = ReadHelper::read_options(reader, endian, ())?;
    let compressed_data = helper.content;
    let decompressed_data = if header.is_compressed() {
        let mut compressed_data = leading_props.chain(Cursor::new(compressed_data));
        let mut decompressed_data = Vec::new();
        lzma_decompress_with_options(&mut compressed_data, &mut decompressed_data, &lzma_rs::decompress::Options { 
            unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(Some(header.data_size as u64)), 
            memlimit: None,
            allow_incomplete: false 
        }).map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
        decompressed_data
    }
    else { compressed_data };
    <_>::read_options(&mut Cursor::new(decompressed_data), endian, (header,))
}

// need fix header inner
#[binrw::writer(writer, endian)]
fn replay_writer(body: &ReplayBody, header: &ReplayHeader) -> BinResult<()> {
    let mut decompressed_data = Cursor::new(Vec::new());
    body.write_options(&mut decompressed_data, endian, (header,))?;
    let compressed_data = if header.is_compressed() {
        let mut compressed_data = Cursor::new(Vec::new());
        decompressed_data.set_position(0);
        lzma_compress_with_options(&mut decompressed_data, &mut compressed_data, &lzma_rs::compress::Options { 
            unpacked_size: lzma_rs::compress::UnpackedSize::SkipWritingToHeader
        })?;
        let mut data = compressed_data.into_inner();
        data.drain(..5); // replay_parser prepends header.props[0..5]; strip encoder's lzma header
        data
    } else { decompressed_data.into_inner() };
    compressed_data.write_options(writer, endian, ())
}

mod test {
    #![allow(unused_imports)]

    use std::io::Cursor;
    use binrw::BinRead;
    use crate::data::Replay;

    #[test]
    #[ignore]
    fn test_deserialize_replay() {
       let arr = std::fs::read("/Users/iami/Downloads/极羽光_vs_爱尔琳妮_20260531225205_G1.yrp").unwrap();
       let mut reader = Cursor::new(arr);
       let replay = Replay::read_le(&mut reader);
       println!("{:?}", replay);
    }

    #[test]
    fn test_replay_roundtrip() {
        use binrw::BinWrite;
        use crate::data::ReplayHeader;
        use crate::data::ReplayHeaderFlags;
        use crate::data::ReplayVersion;
        use crate::data::ReplayBody;
        use crate::data::ReplayDeck;
        use crate::data::ReplayData;
        use crate::data::Response;
        use crate::data::DuelOptions;
        use crate::utils::string::FixedLengthString;

        let mut original = Replay {
            header: ReplayHeader {
                id: ReplayVersion::V2 as u32,
                version: 0x1362,
                flag: ReplayHeaderFlags::Uniform | ReplayHeaderFlags::Compressed,
                seed: 0,
                data_size: 0,
                start_time: 1234567890,
                props: [93, 0, 0, 128, 0, 0, 0, 0],
                seed_sequence: [1, 2, 3, 4, 5, 6, 7, 8],
                header_version: 1,
                reserved: [0; 3],
            },
            body: ReplayBody {
                host_name: FixedLengthString::new("Host".to_string()),
                client_name: FixedLengthString::new("Client".to_string()),
                tag_host_name: None,
                tag_client_name: None,
                start_lp: 8000,
                start_hand: 5,
                draw_count: 1,
                duel_options: DuelOptions::empty(),
                duel_rule: 5,
                host_deck: ReplayDeck::default(),
                client_deck: ReplayDeck::default(),
                tag_host_deck: None,
                tag_client_deck: None,
                datas: vec![ReplayData { data: Response::Unknown(vec![1, 2, 3]) }],
            },
        };
        original.fill_data_size();

        let mut buf = Cursor::new(Vec::new());
        original.write_le(&mut buf).unwrap();
        let written = buf.into_inner();

        let decoded = Replay::read_le(&mut Cursor::new(written)).unwrap();
        assert_eq!(decoded.header.id, original.header.id);
        assert_eq!(decoded.header.version, original.header.version);
        assert_eq!(decoded.header.flag, original.header.flag);
        assert_eq!(decoded.header.data_size, original.header.data_size);
        assert_eq!(decoded.header.seed_sequence, original.header.seed_sequence);
        assert_eq!(decoded.body.host_name.to_string(), original.body.host_name.to_string());
        assert_eq!(decoded.body.client_name.to_string(), original.body.client_name.to_string());
        assert_eq!(decoded.body.start_lp, original.body.start_lp);
        assert_eq!(decoded.body.duel_rule, original.body.duel_rule);
        assert_eq!(decoded.body.datas.len(), original.body.datas.len());
    }
}
