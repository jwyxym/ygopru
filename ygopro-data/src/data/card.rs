use std::ops::Deref;
use std::ops::DerefMut;

#[cfg(feature = "card")]
use rusqlite::Connection;
#[cfg(feature = "card")]
use rusqlite::Row;

use crate::constants::*;


const SIZE_SETCODE: usize = 16;
const SIZE_DESC: usize = 16;

#[repr(C)]
#[derive(Clone, Default, Debug)]
pub struct CoreCard {
    pub code: u32,
    pub alias: u32,
    pub setcode: [u16; SIZE_SETCODE],
    pub card_type: Type,
    pub level: u32,
    pub attribute: Attribute,
    pub race: Race,
    pub attack: i32,
    pub defense: i32,
    pub left_scale: u32,
    pub right_scale: u32,
    pub link_marker: Linkmarkers,
    pub rule_code: u32,
}

impl CoreCard {
    pub fn original_code(&self) -> u32 {
        if self.alias != 0 { self.alias } else { self.code }
    }

    pub fn duel_code(&self) -> u32 {
        if self.rule_code != 0 { self.rule_code } else { self.original_code() }
    }

    pub fn is_setcodes(&self, value: u32) -> bool {
        for x in &self.setcode {
            if *x == 0 { return false; }
            if check_setcode(*x as u32, value) {
                return true;
            }
        }
        false
    }
}

pub fn check_setcode(setcode: u32, value: u32) -> bool {
    setcode > 0 && 
        (setcode & 0x0fffu32) == (value & 0x0fffu32) && 
        (setcode & (value & 0xf000u32)) == (value & 0xf000u32)
}

#[derive(Clone, Default, Debug)]
pub struct Card {
    pub card: CoreCard,
    pub ot: OT,
    pub category: Category,
    pub name: String,
    pub text: String,
    pub desc: [String; SIZE_DESC],
}

impl Deref for Card {
    type Target = CoreCard;

    fn deref(&self) -> &Self::Target {
        &self.card
    }
}

impl DerefMut for Card {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.card
    }
}

#[cfg(feature = "card")]
impl<'row, 'stmt> TryFrom<&'row Row<'stmt>> for CoreCard {
    type Error = rusqlite::Error;

    fn try_from(row: &'row Row<'stmt>) -> Result<Self, Self::Error> {
        let level_raw: u32 = row.get(7)?;
        let defense_raw: i32 = row.get(6)?;
        let setcode_raw: i64 = row.get(3)?;
        let card_type: Type = Type::from_bits_retain(row.get(4)?);
        let is_link = card_type.contains(Type::Link);
        Ok(CoreCard {
            code: row.get(0)?,
            alias: row.get(2)?,
            setcode: {
                let mut sc = [0u16; SIZE_SETCODE];
                sc[0] = (setcode_raw & 0xFFFF) as u16;
                sc[1] = ((setcode_raw >> 16) & 0xFFFF) as u16;
                sc[2] = ((setcode_raw >> 32) & 0xFFFF) as u16;
                sc[3] = ((setcode_raw >> 48) & 0xFFFF) as u16;
                sc
            },
            card_type,
            level: level_raw & 0xFF,
            attribute: Attribute::from_bits_retain(row.get(9)?),
            race: Race::from_bits_retain(row.get(8)?),
            attack: row.get(5)?,
            defense: defense_raw,
            left_scale: (level_raw >> 24) & 0xFF,
            right_scale: (level_raw >> 16) & 0xFF,
            link_marker: if is_link {
                Linkmarkers::from_bits_retain(defense_raw as u32)
            } else {
                Linkmarkers::empty()
            },
            rule_code: 0,
        })
    }
}

#[cfg(feature = "card")]
impl<'row, 'stmt> TryFrom<&'row Row<'stmt>> for Card {
    type Error = rusqlite::Error;

    fn try_from(row: &'row Row<'stmt>) -> Result<Self, Self::Error> {
        Ok(Card {
            card: CoreCard::try_from(row)?,
            ot: OT::from_bits_retain(row.get::<_, i64>(1)? as u8),
            category: Category::from_bits_retain(row.get::<_, i64>(10)? as u32),
            name: row.get(11)?,
            text: row.get(12)?,
            desc: [
                row.get(13)?,
                row.get(14)?,
                row.get(15)?,
                row.get(16)?,
                row.get(17)?,
                row.get(18)?,
                row.get(19)?,
                row.get(20)?,
                row.get(21)?,
                row.get(22)?,
                row.get(23)?,
                row.get(24)?,
                row.get(25)?,
                row.get(26)?,
                row.get(27)?,
                row.get(28)?,
            ],
        })
    }
}

#[cfg(feature = "card")]
pub fn load_db<C>(connection: Connection) -> Result<Vec<C>, rusqlite::Error>
where
    C: for<'row, 'stmt> TryFrom<&'row Row<'stmt>, Error = rusqlite::Error>,
{
    let query = concat!(
        "SELECT d.id, d.ot, d.alias, d.setcode, d.type, d.atk, d.def, d.level, d.race, d.attribute, d.category,",
        " t.name, t.desc, t.str1, t.str2, t.str3, t.str4, t.str5, t.str6, t.str7, t.str8,",
        " t.str9, t.str10, t.str11, t.str12, t.str13, t.str14, t.str15, t.str16",
        " FROM datas d LEFT JOIN texts t ON d.id = t.id",
    );
    let mut stmt = connection.prepare(query)?;
    let res = stmt.query_map([], |row| C::try_from(row))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(res)
}

#[cfg(feature = "card")]
pub fn load_db_from_file<C>(file: &str) -> Result<Vec<C>, rusqlite::Error>
where
    C: for<'row, 'stmt> TryFrom<&'row Row<'stmt>, Error = rusqlite::Error>,
{
    let connection = Connection::open(file)?;
    load_db(connection)
}

#[cfg(feature = "card")]
pub fn load_db_from_bytes<C>(bytes: &[u8]) -> Result<Vec<C>, rusqlite::Error>
where
    C: for<'row, 'stmt> TryFrom<&'row Row<'stmt>, Error = rusqlite::Error>,
{
    let mut connection = Connection::open_in_memory()?;
    let mut cursor = std::io::Cursor::new(bytes);
    connection.deserialize_read_exact("main", &mut cursor, bytes.len(), true)?;
    load_db(connection)
}

mod test {
    #![allow(unused_imports)]

    use crate::data::Card;
    use crate::data::CoreCard;
    use crate::constants::*;
    use crate::data::card::SIZE_SETCODE;

    #[test]
    fn validate_core_card_raw_bytes() {
        let card = CoreCard {
            code: 0xAABBCCDD,
            alias: 0x11223344,
            setcode: {
                let mut sc = [0u16; SIZE_SETCODE];
                sc[0] = 0x5566;
                sc[1] = 0x7788;
                sc[2] = 0x99aa;
                sc
            },
            card_type: Type::Monster | Type::Effect,
            level: 8,
            attribute: Attribute::Dark,
            race: Race::Dragon,
            attack: 3000,
            defense: 2500,
            left_scale: 4,
            right_scale: 4,
            link_marker: Linkmarkers::Bottom | Linkmarkers::Top,
            rule_code: 0xDEADBEEF,
        };
        unsafe {
            let p = &card as *const CoreCard as *const u8;
            let n = std::mem::size_of::<CoreCard>();
            let bytes = std::slice::from_raw_parts(p, n);
            assert_eq!(
                bytes,
                &[
                    0xdd, 0xcc, 0xbb, 0xaa, // code
                    0x44, 0x33, 0x22, 0x11, // alias
                    // setcode[0..16] = 32 bytes
                    0x66, 0x55, 0x88, 0x77, 0xaa, 0x99, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x21, 0x00, 0x00, 0x00, // card_type
                    0x08, 0x00, 0x00, 0x00, // level
                    0x20, 0x00, 0x00, 0x00, // attribute
                    0x00, 0x20, 0x00, 0x00, // race
                    0xb8, 0x0b, 0x00, 0x00, // attack
                    0xc4, 0x09, 0x00, 0x00, // defense
                    0x04, 0x00, 0x00, 0x00, // left_scale
                    0x04, 0x00, 0x00, 0x00, // right_scale
                    0x82, 0x00, 0x00, 0x00, // link_marker
                    0xef, 0xbe, 0xad, 0xde, // rule_code
                ][..]
            );
        }
    }
}
