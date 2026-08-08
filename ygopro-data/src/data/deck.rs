use std::collections::HashMap;
use std::fmt::Display;

use binrw::BinRead;
use binrw::BinWrite;
use binrw::binrw;
use modular_bitfield::Specifier;
use modular_bitfield::bitfield;
use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;

use crate::constants::OT;
use crate::constants::Rule;
use crate::constants::Type;
use crate::data::Card;
use crate::data::LFList;

const DECK_MIN: usize = 40;
const DECK_MAX: usize = 60;
const EXTRA_MAX: usize = 15;
const SIDE_MAX: usize = 15;

#[binrw]
#[derive(Debug, Clone, Default)]
pub struct Deck {
    #[bw(calc = main.len() as u32 + extra.len() as u32)]
    main_size: u32,
    #[bw(calc = side.len() as u32)]
    side_size: u32,
    #[br(count = main_size)]
    pub main: Vec<u32>,
    #[br(count = side_size)]
    pub side: Vec<u32>,
    #[br(ignore)]
    pub extra: Vec<u32>,
}

impl Deck {
    pub fn new() -> Self { Self::default() }

    pub fn load_from_codes(codes: &[u32], mainc: usize, sidec: usize) -> Self {
        let mut d = Self::new();
        let mc = mainc.min(codes.len());
        d.main.extend_from_slice(&codes[..mc]);
        let sc = sidec.min(codes.len().saturating_sub(mc));
        d.side.extend_from_slice(&codes[mc..mc + sc]);
        d
    }

    pub fn get_hash(&self) -> HashMap<u32, usize> {
        let mut counts: HashMap<u32, usize> = HashMap::new();
        for &code in self.main.iter().chain(self.extra.iter()).chain(self.side.iter()) {
            *counts.entry(code).or_insert(0) += 1;
        }
        counts 
    }

    pub fn load<'a>(&mut self, resolve_card: impl Fn(u32) -> Option<&'a Card>) -> Option<DeckError> {
        let response = remove_unknown_cards(&mut self.main, |c| resolve_card(c).map(|c| c.card_type))
            .or(remove_unknown_cards(&mut self.side, |c| resolve_card(c).map(|c| c.card_type)));
        self.separate(|c| resolve_card(c).map(|c| c.card_type).unwrap_or(Type::empty()));
        response
    }

    pub fn prepare<'a>(&mut self, lflist: &LFList, rule: Rule, resolve_card: impl Fn(u32) -> Option<&'a Card>) -> Result<(), DeckError> {
        self.check(&lflist.content, rule, 
            |c| resolve_card(c).map(|c| c.ot).unwrap_or(OT::empty()), 
            |c| resolve_card(c).map(|c| c.card_type).unwrap_or(Type::empty()),
            |c| resolve_card(c).map(|c| c.duel_code()).unwrap_or(0))
    }

    pub fn check_after_replacing_side<'a>(&self, deck: &mut Deck, resolve_card: impl Fn(u32) -> Option<&'a Card>) -> Result<(), DeckError> {
        deck.separate(|c| resolve_card(c).map(|c| c.card_type).unwrap_or(Type::empty()));
        if self == deck {
            Ok(())
        } else {
            Err(DeckError::new().with_error_type(DeckErrorType::SideCount))
        }
    }

    pub fn separate(&mut self, resolve_type: impl Fn(u32) -> Type) {
        separate_main_and_extra(&mut self.main, &mut self.extra, resolve_type);
    }

    pub fn check(&self, lflist: &HashMap<u32, u8>, rule: Rule, get_rule: impl Fn(u32) -> OT, get_type: impl Fn(u32) -> Type, resolve_code: impl Fn(u32) -> u32) -> Result<(), DeckError> {
        check_deck_length(&self.main, &self.extra, &self.side)?;
        check_illegal_cards(&self.main, &self.side, &self.extra, get_type)?;
        let iter = self.main.iter().chain(self.extra.iter()).chain(self.side.iter());
        check_rule(iter.clone(), rule, get_rule)?;
        check_deck_lflists(iter, lflist, resolve_code)
    }
}

impl PartialEq for Deck {
    fn eq(&self, other: &Self) -> bool {
        if self.main.len() != other.main.len() 
            || self.side.len() != other.side.len()
            || self.extra.len() != other.extra.len() {
            return false;
        }
        self.get_hash() == other.get_hash()
    }
}

impl Eq for Deck {}

#[derive(Specifier, Clone, Copy, Debug, IntoPrimitive, TryFromPrimitive, PartialEq, Eq)]
#[bits = 4]
#[repr(u8)]
pub enum DeckErrorType {
    Lflist = 0x1,
    OcgOnly = 0x2,
    TcgOnly = 0x3,
    UnknownCard = 0x4,
    CardCount = 0x5,
    MainCount = 0x6,
    ExtraCount = 0x7,
    SideCount = 0x8,
    NotAvailable = 0x9,
}

#[bitfield]
#[derive(BinRead, BinWrite, Debug, Clone, Copy, PartialEq, Eq)]
#[br(map = Self::from_bytes)]
#[bw(map = |&x| Self::into_bytes(x))]
#[repr(u32)]
pub struct DeckError {
    code: modular_bitfield::specifiers::B28,
    error_type: DeckErrorType,
}

impl Display for DeckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DeckError({:?}, code: {})", self.error_type(), self.code())
    }
}

impl std::error::Error for DeckError {}

const EXTRA_TYPE: Type = Type::from_bits_retain(0x4802040);

pub fn separate_main_and_extra(main: &mut Vec<u32>, ex: &mut Vec<u32>, resolve_type: impl Fn(u32) -> Type) {
    main.retain(|&code| {
        if resolve_type(code).intersects(EXTRA_TYPE) {
            if ex.len() < EXTRA_MAX { ex.push(code); }
            false
        } else {
            true
        }
    });
}

pub fn check_deck_length(main: &[u32], extra: &[u32], side: &[u32]) -> Result<(),DeckError> {
    if main.len() < DECK_MIN || main.len() > DECK_MAX { return Err(DeckError::new().with_error_type(DeckErrorType::MainCount).with_code(main.len() as u32)); }
    if extra.len() > EXTRA_MAX { return Err(DeckError::new().with_error_type(DeckErrorType::ExtraCount).with_code(extra.len() as u32)); }
    if side.len() > SIDE_MAX { return Err(DeckError::new().with_error_type(DeckErrorType::SideCount).with_code(side.len() as u32)); }
    Ok(())
}

pub fn remove_unknown_cards(main: &mut Vec<u32>, get_type: impl Fn(u32) -> Option<Type>) -> Option<DeckError> {
    let mut last_removed_code = None;
    main.retain(|code| {
        let _type = get_type(*code);
        if match _type {
            Some(_type) => _type.contains(Type::Token),
            None => true
        } {
            last_removed_code = Some(*code);
            false
        } else { true }
    });
    last_removed_code.map(|code| DeckError::new().with_error_type(DeckErrorType::UnknownCard).with_code(code))
}

pub fn check_illegal_cards(main: &Vec<u32>, side: &Vec<u32>, ex: &Vec<u32>, get_type: impl Fn(u32) -> Type) -> Result<(), DeckError> {
    for code in main {
        let card_type = get_type(*code);
        if card_type.contains(Type::Token) || card_type.intersects(EXTRA_TYPE) {
            return Err(DeckError::new().with_error_type(DeckErrorType::MainCount).with_code(0));
        }
    }
    for code in side {
        if get_type(*code).contains(Type::Token) {
            return Err(DeckError::new().with_error_type(DeckErrorType::SideCount).with_code(0));
        }
    }
    for code in ex {
        let card_type = get_type(*code);
        if card_type.contains(Type::Token) || !card_type.intersects(EXTRA_TYPE) {
            return Err(DeckError::new().with_error_type(DeckErrorType::ExtraCount).with_code(0));
        }
    }
    Ok(())
}

pub fn check_rule<'a>(codes: impl Iterator<Item = &'a u32>, rule: Rule, get_rule: impl Fn(u32) -> OT) -> Result<(), DeckError> {
    for &code in codes {
        let ot = get_rule(code);
        if let Some(error_type) = rule.check_ot(ot) {
            return Err(DeckError::new().with_error_type(error_type).with_code(code));
        }
    }
    Ok(())
}

pub fn check_deck_lflists<'a>(codes: impl Iterator<Item = &'a u32>, lflist: &HashMap<u32, u8>, resolve_code: impl Fn(u32) -> u32) -> Result<(), DeckError> {
    let mut counts: HashMap<u32, u32> = HashMap::new();
    for &code in codes {
        let resolved = resolve_code(code);
        *counts.entry(resolved).or_insert(0) += 1;
    }

    for (&code, &count) in &counts {
        if let Some(&limit) = lflist.get(&code) {
            if count as u8 > limit { return Err(DeckError::new().with_error_type(DeckErrorType::Lflist).with_code(code)); }
        }
        if count > 3 { return Err(DeckError::new().with_error_type(DeckErrorType::CardCount).with_code(code)); }
    }
    Ok(())
}

