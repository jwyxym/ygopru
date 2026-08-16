use std::io::prelude::Read;
use std::io::prelude::Write;
use std::io::prelude::Seek;

use binrw::BinRead;
use binrw::BinReaderExt;
use binrw::BinWrite;
use binrw::VecArgs;

use crate::constants::*;
use crate::message::game_message::CardCode;
use crate::message::game_message::GameMessage;
use ygopro_derive::GameMessage;

#[derive(BinRead, BinWrite, Clone, Debug, GameMessage)]
pub struct CardPosition<const CODE: bool, const SUB_SEQUENCE: bool, const DESCRIPTION: bool> {
    #[brw(if(CODE))]
    #[mask]
    #[mask_if(self.controller != player)]
    pub code: CardCode,
    pub controller: CorePlayer,
    pub location: Location,
    pub sequence: i8,
    #[brw(if(SUB_SEQUENCE))]
    pub sub_sequence: i8,
    #[brw(if(DESCRIPTION))]
    pub description: i32
}

// In ygopro, these fields are packed as a u32.
// We don't need pack them.
#[derive(BinRead, BinWrite, Debug, Copy, Clone, PartialEq, Eq)]
pub struct InfoLocation {
    pub controller: CorePlayer,
    pub location: Location,
    pub sequence: u8,
    #[br(if(!location.contains(Location::Overlay), Position::Any))]
    #[bw(if(!location.contains(Location::Overlay)))]
    pub position: Position,
    #[brw(if(location.contains(Location::Overlay)))]
    pub overlay_sequence: u8
}

impl InfoLocation {
    pub fn should_mask(&self) -> bool {
        if self.position.intersects(Position::Reveal) {
            return false;
        }
        if self.location.contains(Location::Hand) {
            !self.position.intersects(Position::Faceup)
        } else {
            self.position.intersects(Position::Facedown)
        }
    }
}

#[derive(Clone, Debug)]
pub enum QueryData {
    Clear,
    Code(i32),
    Position(InfoLocation),
    Alias(i32),
    Type(Type),
    Level(i32),
    Rank(i32),
    Attribute(Attribute),
    Race(Race),
    Attack(i32),
    Defense(i32),
    BaseAttack(i32),
    BaseDefense(i32),
    Reason(Reason),
    ReasonCard(i32),
    EquipCard(CardPosition<false, true, false>),
    TargetCard(Vec<CardPosition<false, true, false>>),
    OverlayCard(Vec<u32>),
    Counters(Vec<(u16, u16)>),
    Owner(CorePlayer),
    Status(Status),
    LeftScale(i32),
    RightScale(i32),
    Link(i32, Linkmarkers)
}

pub(crate) struct QueryDatas(Vec<QueryData>);

impl BinRead for QueryDatas {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(reader: &mut R, endian: binrw::Endian, _: Self::Args<'_>,) -> binrw::prelude::BinResult<Self> {
        let query = Query::read_options(reader, endian, ())?;
        let mut query_datas = Vec::new();
        if query.is_empty() { query_datas.push(QueryData::Clear); }
        if query.contains(Query::Code) { query_datas.push(QueryData::Code(i32::read_options(reader, endian, ())?)); }
        if query.contains(Query::Position) { 
            query_datas.push(QueryData::Position(InfoLocation::read_options(reader, endian, ())?));
        }
        if query.contains(Query::Alias)       { query_datas.push(QueryData::Alias(i32::read_options(reader,           endian, ())?)); }
        if query.contains(Query::Type)        { query_datas.push(QueryData::Type(Type::read_options(reader,           endian, ())?)); }
        if query.contains(Query::Level)       { query_datas.push(QueryData::Level(i32::read_options(reader,           endian, ())?)); }
        if query.contains(Query::Rank)        { query_datas.push(QueryData::Rank(i32::read_options(reader,            endian, ())?)); }
        if query.contains(Query::Attribute)   { query_datas.push(QueryData::Attribute(Attribute::read_options(reader, endian, ())?)); }
        if query.contains(Query::Race)        { query_datas.push(QueryData::Race(Race::read_options(reader,           endian, ())?)); }
        if query.contains(Query::Attack)      { query_datas.push(QueryData::Attack(i32::read_options(reader,          endian, ())?)); }
        if query.contains(Query::Defense)     { query_datas.push(QueryData::Defense(i32::read_options(reader,         endian, ())?)); }
        if query.contains(Query::BaseAttack)  { query_datas.push(QueryData::BaseAttack(i32::read_options(reader,      endian, ())?)); }
        if query.contains(Query::BaseDefense) { query_datas.push(QueryData::BaseDefense(i32::read_options(reader,     endian, ())?)); }
        if query.contains(Query::Reason)      { query_datas.push(QueryData::Reason(Reason::read_options(reader,       endian, ())?)); }
        if query.contains(Query::ReasonCard)  { query_datas.push(QueryData::ReasonCard(i32::read_options(reader,      endian, ())?)); }
        if query.contains(Query::EquipCard)   { query_datas.push(QueryData::EquipCard(CardPosition::<false,true,false>::read_options(reader, endian, ())?)); }
        if query.contains(Query::TargetCard) {
            let count = u32::read_options(reader, endian, ())? as usize;
            query_datas.push(QueryData::TargetCard(Vec::<CardPosition<false, true, false>>::read_options(reader, endian, VecArgs { count, inner: () })?));
        }
        if query.contains(Query::OverlayCard) {
            let count = u32::read_options(reader, endian, ())? as usize;
            query_datas.push(QueryData::OverlayCard(Vec::<u32>::read_options(reader, endian, VecArgs { count, inner: () })?)); 
        }
        if query.contains(Query::Counters) { 
            let count = u32::read_options(reader, endian, ())? as usize;
            query_datas.push(QueryData::Counters(Vec::<(u16, u16)>::read_options(reader, endian, VecArgs { count, inner: () })?)); 
        }
        if query.contains(Query::Owner) { 
            query_datas.push(QueryData::Owner(CorePlayer::read_options(reader, endian, ())?)); 
            reader.read_le::<[u8; 3]>()?; // padding 3 bytes
        }
        if query.contains(Query::Status)     { query_datas.push(QueryData::Status(Status::read_options(reader,  endian, ())?)); }
        if query.contains(Query::LeftScale)  { query_datas.push(QueryData::LeftScale(i32::read_options(reader,  endian, ())?)); }
        if query.contains(Query::RightScale) { query_datas.push(QueryData::RightScale(i32::read_options(reader, endian, ())?)); }
        if query.contains(Query::Link)       { query_datas.push(QueryData::Link(i32::read_options(reader, endian, ())?, Linkmarkers::read_options(reader, endian, ())?)); }
        Ok(QueryDatas(query_datas))
    }
}

impl<'a> From<&'a QueryData> for Query {
    fn from(value: &'a QueryData) -> Self {
        match value {
            QueryData::Clear          => Query::empty(),
            QueryData::Code(_)        => Query::Code,
            QueryData::Position(_)    => Query::Position,
            QueryData::Alias(_)       => Query::Alias,
            QueryData::Type(_)        => Query::Type,
            QueryData::Level(_)       => Query::Level,
            QueryData::Rank(_)        => Query::Rank,
            QueryData::Attribute(_)   => Query::Attribute,
            QueryData::Race(_)        => Query::Race,
            QueryData::Attack(_)      => Query::Attack,
            QueryData::Defense(_)     => Query::Defense,
            QueryData::BaseAttack(_)  => Query::BaseAttack,
            QueryData::BaseDefense(_) => Query::BaseDefense,
            QueryData::Reason(_)      => Query::Reason,
            QueryData::ReasonCard(_)  => Query::ReasonCard,
            QueryData::EquipCard(_)   => Query::EquipCard,
            QueryData::TargetCard(_)  => Query::TargetCard,
            QueryData::OverlayCard(_) => Query::OverlayCard,
            QueryData::Counters(_)    => Query::Counters,
            QueryData::Owner(_)       => Query::Owner,
            QueryData::Status(_)      => Query::Status,
            QueryData::LeftScale(_)   => Query::LeftScale,
            QueryData::RightScale(_)  => Query::RightScale,
            QueryData::Link(_, _)        => Query::Link,
        }
    }
}

#[derive(Clone, Debug)]
pub enum UpdateCardInfo {
    Empty,
    Data(Vec<QueryData>)
}

impl GameMessage for UpdateCardInfo {
    fn mask(&mut self) {
        if !self.should_mask(CorePlayer::None) { return }
        if let UpdateCardInfo::Data(data) = self {
            data.fill(QueryData::Clear);
        }
    }

    fn should_mask(&self, _player: CorePlayer) -> bool {
        let data = match self {
            UpdateCardInfo::Data(data) => data,
            _ => return false
        };
        data.iter().find_map(|q| if let QueryData::Position(p) = q { 
            Some(p.should_mask()) 
        } else { None }).unwrap_or(false)
    }
}

impl BinRead for UpdateCardInfo {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(reader: &mut R, endian: binrw::Endian, _: Self::Args<'_>) -> binrw::prelude::BinResult<Self> {
        let len = u32::read_options(reader, endian, ())?;
        if len == 4 { return Ok(UpdateCardInfo::Empty); }
        let pos = reader.stream_position()?;
        let datas = QueryDatas::read_options(reader, endian, ())?;
        reader.seek(std::io::SeekFrom::Start(pos + len as u64 - 4))?;
        Ok(UpdateCardInfo::Data(datas.0))
    }
}

impl BinWrite for UpdateCardInfo {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(&self, writer: &mut W, endian: binrw::Endian, _: Self::Args<'_>) -> binrw::prelude::BinResult<()> {
        let args = ();
        let queries = match self {
            UpdateCardInfo::Empty => return u32::write_options(&4, writer, endian, args),
            UpdateCardInfo::Data(data) => data
        };
        let flag: u32 = queries.iter().map(|query| Query::from(query).bits()).sum();
        let mut len = 0u32;
        let pos = writer.stream_position()?;
        len.write_options(writer, endian, ())?;
        flag.write_options(writer, endian, ())?;
        for query in queries {
            match query {
                QueryData::Clear                => (),
                QueryData::Code(code)           => code.write_options(writer,     endian, args)?,
                QueryData::Position(info_location)   => info_location.write_options(writer, endian, args)?,
                QueryData::Alias(alias)         => alias.write_options(writer, endian, args)?,
                QueryData::Type(_type)          => _type.write_options(writer, endian, args)?,
                QueryData::Level(level)         => level.write_options(writer, endian, args)?,
                QueryData::Rank(rank)           => rank.write_options(writer, endian, args)?,
                QueryData::Attribute(attribute) => attribute.write_options(writer, endian, args)?,
                QueryData::Race(race)           => race.write_options(writer, endian, args)?,
                QueryData::Attack(attack)       => attack.write_options(writer, endian, args)?,
                QueryData::Defense(defense)     => defense.write_options(writer, endian, args)?,
                QueryData::BaseAttack(attack)      => attack.write_options(writer, endian, args)?,
                QueryData::BaseDefense(defense)     => defense.write_options(writer, endian, args)?,
                QueryData::Reason(reason)       => reason.write_options(writer, endian, args)?,
                QueryData::ReasonCard(card)     => card.write_options(writer, endian, args)?,
                QueryData::EquipCard(card)      => card.write_options(writer, endian, args)?,
                QueryData::TargetCard(cards)    => {
                    let len = cards.len() as u32;
                    len.write_options(writer, endian, args)?;
                    cards.write_options(writer, endian, args)?;
                },
                QueryData::OverlayCard(cards)   => {
                    let len = cards.len() as u32;
                    len.write_options(writer, endian, args)?;
                    cards.write_options(writer, endian, args)?;
                },
                QueryData::Counters(counters)   => {
                    let len = counters.len() as u32;
                    len.write_options(writer, endian, args)?;
                    counters.write_options(writer, endian, args)?;
                },
                QueryData::Owner(owner)         => {
                    owner.write_options(writer, endian, args)?;
                    [0u8; 3].write_options(writer, endian, args)?;
                },
                QueryData::Status(status)       => status.write_options(writer, endian, args)?,
                QueryData::LeftScale(scale)     => scale.write_options(writer, endian, args)?,
                QueryData::RightScale(scale)    => scale.write_options(writer, endian, args)?,
                QueryData::Link(link, linkmarkers) => {
                    link.write_options(writer, endian, args)?;
                    linkmarkers.write_options(writer, endian, args)?
                },
            }
        }
        let current_pos = writer.stream_position()?;
        len = u32::try_from(current_pos - pos).map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
        writer.seek(std::io::SeekFrom::Current(-(len as i64)))?;
        len.write_options(writer, endian, args)?;
        writer.seek(std::io::SeekFrom::Current((len - 4) as i64))?;
        Ok(())
    }
}

mod test {
    #![allow(unused_imports)]

    use std::io::Cursor;
    use binrw::{BinRead, BinWrite, VecArgs};
    use crate::data::UpdateCardInfo;

    #[test]
    fn test_deserialize_query() {
        let arr = vec![16, 0, 0, 0, 3, 0, 0, 0, 122, 178, 159, 1, 1, 2, 0, 10, 16, 0, 0, 0, 3, 0, 0, 0, 17, 231, 97, 3, 1, 2, 1, 10, 16, 0, 0, 0, 3, 0, 0, 0, 59, 73, 201, 5, 1, 2, 2, 10, 16, 0, 0, 0, 3, 0, 0, 0, 239, 39, 81, 0, 1, 2, 3, 10, 16, 0, 0, 0, 3, 0, 0, 0, 143, 77, 182, 3, 1, 2, 4, 1];
        let re: Vec<u8> = vec![16, 0, 0, 0, 3, 0, 0, 0, 122, 178, 159, 1, 1, 2, 0, 10, 16, 0, 0, 0, 3, 0, 0, 0, 17, 231, 97, 3, 1, 2, 1, 10, 16, 0, 0, 0, 3, 0, 0, 0, 59, 73, 201, 5, 1, 2, 2, 10, 16, 0, 0, 0, 3, 0, 0, 0, 239, 39, 81, 0, 1, 2, 3, 10, 16, 0, 0, 0, 3, 0, 0, 0, 143, 77, 182, 3, 1, 2, 4, 1];
        let mut reader = Cursor::new(arr);
        let replay = Vec::<UpdateCardInfo>::read_le_args(&mut reader, VecArgs {count: 5, inner: {}}).unwrap();
        let mut writer = Cursor::new(Vec::new());
        replay.write_le(&mut writer).unwrap();
        let result = writer.into_inner();
        assert_eq!(result, re, "round-trip mismatch");
    }
}
