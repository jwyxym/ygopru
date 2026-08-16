#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]

use binrw::BinRead;
use binrw::BinWrite;
use binrw::binrw;
use binrw::helpers::until_eof;
use modular_bitfield::bitfield;
use modular_bitfield::specifiers::B3;
use modular_bitfield::specifiers::B28;
use num_enum::TryFromPrimitive;
use ygopro_derive::GameMessage;
use ygopro_derive::Message;

use crate::constants::*;
use crate::data::CardPosition;
use crate::data::InfoLocation;
use crate::data::UpdateCardInfo;

include!(concat!(env!("OUT_DIR"), "/game_message.rs"));
every_game_message_flat_message!(crate::generate_enum);

macro_rules! impl_into_stoc_message {
    ($($message_name:ident=$message_flag:literal),*) => {
        $(
            impl From<$message_name> for crate::message::server_to_client::Message {
                fn from(value: $message_name) -> Self {
                    crate::message::server_to_client::Message::GameMessage(
                        crate::message::server_to_client::GameMessage { message: value.into() }
                    )
                }
            }
        )*
    };
}
every_game_message_flat_message!(impl_into_stoc_message);

impl From<crate::message::game_message::Message> for crate::message::server_to_client::Message {
    fn from(value: crate::message::game_message::Message) -> Self {
        crate::message::server_to_client::Message::GameMessage(
            crate::message::server_to_client::GameMessage { message: value }
        )
    }
}


pub trait GameMessage {
    fn mask(&mut self);
    fn should_mask(&self, _player: CorePlayer) -> bool {
        true
    }
    fn waiting_for(&self) -> Option<CorePlayer> {
        None
    }
}

pub trait MaskedClone: GameMessage + Clone {
    fn clone_masked(&self) -> Self {
        let mut mirror = self.clone();
        mirror.mask();
        mirror
    }
}

impl<T: GameMessage> GameMessage for Vec<T> {
    fn mask(&mut self) {
        for item in self { item.mask(); }
    }
    fn should_mask(&self, player: CorePlayer) -> bool {
        self.iter().any(|item| item.should_mask(player))
    }
}

impl GameMessage for Vec<u32> {
    fn mask(&mut self) {
        self.fill(0);
    }
}

impl<T> MaskedClone for T where T: GameMessage + Clone {}

macro_rules! impl_mask_for_message {
    ($($message_name:ident=$message_flag:literal),*) => {
        impl GameMessage for Message {
            fn mask(&mut self) {
                match self {
                    $(Message::$message_name(inner) => inner.mask()),*
                }
            }
            fn should_mask(&self, player: CorePlayer) -> bool {
                match self {
                    $(Message::$message_name(inner) => inner.should_mask(player)),*
                }
            }
            fn waiting_for(&self) -> Option<CorePlayer> {
                match self {
                    $(Message::$message_name(inner) => inner.waiting_for()),*
                }
            }
        }
    };
}
every_game_message_flat_message!(impl_mask_for_message);

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 1)]
#[repr(C)] 
pub struct Retry;

#[derive(Debug, Message, Clone, GameMessage)]
#[message(gm, flag = 2)]
#[binrw]
#[repr(C)]
pub struct Hint {
    pub _type: crate::constants::Hint,
    pub player: CorePlayer,
    pub data: i32
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 3)]
#[repr(C)]
pub struct Waiting;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 4)]
#[repr(C)]
pub struct Start {
    pub player_type: u8,
    pub rule: MasterRule,
    pub player1_lp: i32,
    pub player2_lp: i32,
    pub player1_deck_count: u16,
    pub player1_extra_count: u16,
    pub player2_deck_count: u16,
    pub player2_extra_count: u16
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 5)]
#[repr(C)]
pub struct Win {
    pub winner: CorePlayer,
    pub reason: WinReason
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 6)]
#[repr(C)]
pub struct UpdateData {
    pub player: CorePlayer,
    pub location: Location,
    #[br(parse_with=until_eof)]
    #[mask]
    #[mask_if(self.player != player)]
    pub data: Vec<UpdateCardInfo>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 7)]
#[repr(C)]
pub struct UpdateCard {
    pub position: CardPosition<false, false, false>,
    #[mask]
    #[mask_if(self.position.controller != player && self.data.should_mask(player))]
    pub data: UpdateCardInfo
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 8)]
#[repr(C)]
pub struct RequestDeck;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 10)]
#[repr(C)]
pub struct SelectBattleCommand {
    #[wait_for]
    pub selecting_player: CorePlayer,
    #[bw(calc(activatable_cards.len() as u8))]
    activatable_cards_size: u8,
    #[br(count = activatable_cards_size)]
    pub activatable_cards: Vec<CardPosition<true, false, true>>,
    #[bw(calc(attackable_cards.len() as u8))]
    attackable_cards_size: u8,
    #[br(count = attackable_cards_size)]
    pub attackable_cards: Vec<(CardPosition<true, false, false>, i8)>, // Diratt
    #[br(map=|v:u8| v>0)]
    #[bw(map=|v| if *v {1u8} else {0u8})]
    pub can_enter_m2: bool, // u8
    #[br(map=|v:u8| v>0)]
    #[bw(map=|v| if *v {1u8} else {0u8})]
    pub can_enter_ep: bool  // u8
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 11)]
#[repr(C)]
pub struct SelectIdleCommand {
    #[wait_for]
    pub selecting_player: CorePlayer,
    #[bw(calc(summonable_cards.len() as u8))]
    summonable_cards_size: u8,
    #[br(count = summonable_cards_size)]
    pub summonable_cards: Vec<CardPosition<true, false, false>>,
    #[bw(calc(special_summonable_cards.len() as u8))]
    special_summonable_cards_size: u8,
    #[br(count = special_summonable_cards_size)]
    pub special_summonable_cards: Vec<CardPosition<true, false, false>>,
    #[bw(calc(repositionable_cards.len() as u8))]
    repositionable_cards_size: u8,
    #[br(count = repositionable_cards_size)]
    pub repositionable_cards: Vec<CardPosition<true, false, false>>,
    #[bw(calc(m_setable_cards.len() as u8))]
    m_setable_cards_size: u8,
    #[br(count = m_setable_cards_size)]
    pub m_setable_cards: Vec<CardPosition<true, false, false>>,
    #[bw(calc(s_setable_cards.len() as u8))]
    s_setable_cards_size: u8,
    #[br(count = s_setable_cards_size)]
    pub s_setable_cards: Vec<CardPosition<true, false, false>>,
    #[bw(calc(activatable_cards.len() as u8))]
    activatable_cards_size: u8,
    #[br(count = activatable_cards_size)]
    pub activatable_cards: Vec<CardPosition<true, false, true>>,
    #[br(map=|v:u8| v>0)]
    #[bw(map=|v| if *v {1u8} else {0u8})]
    pub can_enter_bp: bool, // u8
    #[br(map=|v:u8| v>0)]
    #[bw(map=|v| if *v {1u8} else {0u8})]
    pub can_enter_ep: bool, // u8
    #[br(map=|v:u8| v>0)]
    #[bw(map=|v| if *v {1u8} else {0u8})]
    pub can_shuffle_hand: bool // u8
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 12)]
#[repr(C)]
pub struct SelectEffectYesNo {
    #[wait_for]
    pub selecting_player: CorePlayer,
    pub code: CardCode,
    pub card_position: InfoLocation,
    pub description: i32,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 13)]
#[repr(C)]
pub struct SelectYesNo {
    #[wait_for]
    pub selecting_player: CorePlayer,
    pub description: i32
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 14)]
#[repr(C)]
pub struct SelectOption {
    #[wait_for]
    pub selecting_player: CorePlayer,
    #[bw(calc(options.len() as u8))]
    options_size: u8,
    #[br(count = options_size)]
    pub options: Vec<i32>
}

#[binrw]
#[derive(Debug, Clone, Message)]
#[message(gm, flag = 15)]
#[repr(C)]
pub struct SelectCard {
    pub selecting_player: CorePlayer,
    #[br(map=|v:u8| v>0)]
    #[bw(map=|v| if *v {1u8} else {0u8})]
    pub select_cancelable: bool,
    pub select_min: u8,
    pub select_max: u8,
    #[bw(calc(positions.len() as u8))]
    positions_size: u8,
    #[br(count = positions_size)]
    pub positions: Vec<(CardCode, InfoLocation)>
}

impl GameMessage for SelectCard {
    fn mask(&mut self) {
        for (code, position) in &mut self.positions {
            if position.controller != self.selecting_player {
                code.mask();
            }
        }
    }
    fn should_mask(&self, _player: CorePlayer) -> bool {
        true
    }
    fn waiting_for(&self) -> Option<CorePlayer> {
        Some(self.selecting_player)
    }
}

#[binrw]
#[derive(Debug, Clone)]
pub struct Chain {
    pub flag: u8,
    #[br(map = |v: u8| v != 0)]
    #[bw(map = |v: &bool| *v as u8)]
    pub forced: bool,
    pub code: CardCode,
    pub location: InfoLocation,
    pub description: i32,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 16)]
#[repr(C)]
pub struct SelectChain {
    #[wait_for]
    pub selecting_player: CorePlayer,
    #[bw(calc(activatable_cards.len() as u8))]
    pub activatable_cards_count: u8,
    pub special_count: u8,
    pub hint0: i32,
    pub hint1: i32,
    #[br(count = activatable_cards_count)]
    pub activatable_cards: Vec<Chain>,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 18)]
#[repr(C)]
pub struct SelectPlace {
    #[wait_for]
    pub selecting_player: CorePlayer,
    pub count: u8,
    pub unselectable_field: i32,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 19)]
#[repr(C)]
pub struct SelectPosition {
    #[wait_for]
    pub selecting_player: CorePlayer,
    pub code: u32,
    pub positions: Position
}

#[binrw]
#[derive(Debug, Clone, Message)]
#[message(gm, flag = 20)]
#[repr(C)]
pub struct SelectTribute {
    pub selecting_player: CorePlayer,
    #[br(map=|v:u8| v>0)]
    #[bw(map=|v| if *v {1u8} else {0u8})]
    pub cancelable: bool,
    pub select_min: u8,
    pub select_max: u8,
    #[bw(calc(tributes.len() as u8))]
    tributes_size: u8,
    #[br(count = tributes_size)]
    pub tributes: Vec<(CardPosition<true, false, false>, i8)>
}

impl GameMessage for SelectTribute {
    fn mask(&mut self) {
        for (card_position, _) in &mut self.tributes {
            if card_position.controller != self.selecting_player {
                card_position.code.mask();
            }
        }
    }
    fn should_mask(&self, _player: CorePlayer) -> bool {
        true
    }
    fn waiting_for(&self) -> Option<CorePlayer> {
        Some(self.selecting_player)
    }
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 21)]
#[repr(C)]
pub struct SortChain;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 22)]
#[repr(C)]
pub struct SelectCounter {
    #[wait_for]
    pub selecting_player: CorePlayer,
    pub select_counter_type: u16,
    pub select_counter_count: u16,
    #[bw(calc(selectable_cards.len() as u8))]
    selectable_cards_size: u8,
    #[br(count = selectable_cards_size)]
    pub selectable_cards: Vec<(CardPosition<true, false, false>, i16)>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 23)]
#[repr(C)]
pub struct SelectSum {
    pub select_mode: SelectSumMode,
    #[wait_for]
    pub selecting_player: CorePlayer,
    pub select_sum_value: i32,
    pub select_min: u8,
    pub select_max: u8,
    #[bw(calc(must_select_cards.len() as u8))]
    must_select_cards_size: u8,
    #[br(count = must_select_cards_size)]
    pub must_select_cards: Vec<(CardPosition<true, false, false>, i32)>, // OpParam
    #[bw(calc(select_cards.len() as u8))]
    select_cards_size: u8,
    #[br(count = select_cards_size)]
    pub select_cards: Vec<(CardPosition<true, false, false>, i32)> // OpParam
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 24)]
#[repr(C)]
pub struct SelectDisableField {
    #[wait_for]
    pub selecting_player: CorePlayer,
    pub count: u8,
    pub unselectable_field: i32,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 25)]
#[repr(C)]
pub struct SortCard {
    #[wait_for]
    pub player: CorePlayer,
    #[bw(calc(cards.len() as u8))]
    cards_size: u8,
    #[br(count = cards_size)]
    pub cards: Vec<CardPosition<true, false, false>>
}

#[binrw]
#[derive(Debug, Clone, Message)]
#[message(gm, flag = 26)]
#[repr(C)]
pub struct SelectUnselectCard {
    pub selecting_player: CorePlayer,
    #[br(map=|v:u8| v>0)]
    #[bw(map=|v| if *v {1u8} else {0u8})]
    pub able: bool,
    #[br(map=|v:u8| v>0)]
    #[bw(map=|v| if *v {1u8} else {0u8})]
    pub cancelable: bool,
    pub select_min: u8,
    pub select_max: u8,
    #[bw(calc(positions1.len() as u8))]
    positions1_size: u8,
    #[br(count = positions1_size)]
    pub positions1: Vec<(CardCode, InfoLocation)>,
    #[bw(calc(positions2.len() as u8))]
    positions2_size: u8,
    #[br(count = positions2_size)]
    pub positions2: Vec<(CardCode, InfoLocation)>
}

impl GameMessage for SelectUnselectCard {
    fn mask(&mut self) {
        for (code, position) in &mut self.positions1 {
            if position.controller != self.selecting_player {
                code.mask();
            }
        }
        for (code, position) in &mut self.positions2 {
            if position.controller != self.selecting_player {
                code.mask();
            }
        }
    }
    fn should_mask(&self, _player: CorePlayer) -> bool {
        true
    }
    fn waiting_for(&self) -> Option<CorePlayer> {
        Some(self.selecting_player)
    }
}

#[binrw]
#[derive(Clone, Debug, Message, GameMessage)]
#[message(gm, flag = 30)]
pub struct ConfirmDecktop {
    pub controller: CorePlayer,
    #[bw(calc(codes.len() as u8))]
    codes_size: u8,
    #[br(count = codes_size)]
    pub codes: Vec<CardPosition<true, false, false>>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 31)]
#[repr(C)]
pub struct ConfirmCards {
    pub player: CorePlayer,
    #[br(map=|v:u8| v>0)]
    #[bw(map=|v| if *v {1u8} else {0u8})]
    pub skip_panel: bool, //u8
    #[bw(calc(cards.len() as u8))]
    cards_size: u8,
    #[br(count = cards_size)]
    pub cards: Vec<CardPosition<true, false, false>>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 32)]
#[repr(C)]
pub struct ShuffleDeck {
    pub player: CorePlayer 
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 33)]
#[repr(C)]
pub struct ShuffleHand {
    pub player: CorePlayer,
    #[bw(calc(codes.len() as u8))]
    pub count: u8,
    #[br(count = count)]
    #[mask]
    #[mask_if(self.player != player)]
    pub codes: Vec<u32>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 34)]
#[repr(C)]
pub struct RefreshDeck {
    pub player: CorePlayer
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 35)]
#[repr(C)]
pub struct SwapGraveDeck {
    pub player: CorePlayer 
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 36)]
pub struct ShuffleSetCard {
    pub location: Location,
    #[bw(calc(old_locations.len() as u8), assert(old_locations.len() == new_locations.len(), "ShuffleSetCard: old/new location count mismatch"))]
    count: u8,
    #[br(count = count)]
    pub old_locations: Vec<InfoLocation>,
    #[br(count = count)]
    pub new_locations: Vec<InfoLocation>,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 37)]
#[repr(C)]
pub struct ReverseDeck;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 38)]
#[repr(C)]
pub struct DeckTop {
    pub player: CorePlayer,
    pub sequence: u8,
    pub code: CardCode
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 39)]
#[repr(C)]
pub struct ShuffleExtra {
    pub player: CorePlayer,
    #[bw(calc(codes.len() as u8))]
    pub count: u8,
    #[br(count = count)]
    #[mask]
    #[mask_if(self.player != player)]
    pub codes: Vec<u32>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 40)]
#[repr(C)]
pub struct NewTurn {
    pub player: CorePlayer 
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 41)]
#[repr(C)]
pub struct NewPhase {
    pub phase: crate::constants::Phase,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 42)]
#[repr(C)]
pub struct ConfirmExtraTop {
    pub player: CorePlayer,
    #[bw(calc(selectable_cards.len() as u8))]
    selectable_cards_size: u8,
    #[br(count = selectable_cards_size)]
    pub selectable_cards: Vec<CardPosition<true,false,false>>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 50)]
#[repr(C)]
pub struct Move {
    #[mask(if self.should_mask(CorePlayer::None) { 0 } else { self.code })]
    #[mask_if(self.current.controller != player && !self.current.location.intersects(Location::Grave | Location::Overlay) && (self.current.location.intersects(Location::Deck | Location::Hand) || self.current.position.is_face_down()))]
    pub code: i32,
    pub previous: InfoLocation,
    pub current: InfoLocation,
    pub reason: crate::constants::Reason
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 53)]
#[repr(C)]
pub struct PositionChange {
    pub card: u32,
    pub controller: CorePlayer,
    pub location: Location,
    pub sequence: u8,
    pub previous_position: Position,
    pub current_position: Position
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 54)]
#[repr(C)]
pub struct Set {
    #[mask]
    pub code: i32,
    pub position: InfoLocation
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 55)]
#[repr(C)]
pub struct Swap {
    pub code1: CardCode,
    pub position1: InfoLocation,
    pub code2: CardCode,
    pub position2: InfoLocation,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 56)]
#[repr(C)]
pub struct FieldDisabled {
    pub disabled: i32
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 60)]
#[repr(C)]
pub struct Summoning {
    pub code: u32,
    pub position: InfoLocation,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 61)]
#[repr(C)]
pub struct Summoned;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 62)]
#[repr(C)]
pub struct SpecialSummoning {
    #[mask]
    #[mask_if(self.position.position.is_face_down() && self.position.controller != player)]
    pub code: u32,
    pub position: InfoLocation,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 63)]
#[repr(C)]
pub struct SpecialSummoned;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 64)]
#[repr(C)]
pub struct FlipSummoning {
    pub code: u32,
    pub position: InfoLocation,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 65)]
#[repr(C)]
pub struct FlipSummoned;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 70)]
#[repr(C)]
pub struct Chaining {
    pub card: u32,
    pub previous: CardPosition<false, true, false>,
    pub current: CardPosition<false, false, true>,
    pub chain_count: u8
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 71)]
#[repr(C)]
pub struct Chained {
    pub chain_index: u8
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 72)]
#[repr(C)]
pub struct ChainSolving {
    pub chain_index: u8
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 73)]
#[repr(C)]
pub struct ChainSolved {
    pub chain_index: u8
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 74)]
#[repr(C)]
pub struct ChainEnd;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 75)]
#[repr(C)]
pub struct ChainNegated {
    pub chain_index: u8
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 76)]
#[repr(C)]
pub struct ChainDisabled {
    pub chain_index: u8
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 80)]
#[repr(C)]
pub struct CardSelected {
    pub player: CorePlayer,
    #[bw(calc(pcards.len() as u8))]
    pcards_size: u8,
    #[br(count = pcards_size)]
    pub pcards: Vec<InfoLocation>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 81)]
#[repr(C)]
pub struct RandomSelected {
    pub player: CorePlayer,
    #[bw(calc(pcards.len() as u8))]
    pcards_size: u8,
    #[br(count = pcards_size)]
    pub pcards: Vec<InfoLocation>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 83)]
#[repr(C)]
pub struct BecomeTarget {
    #[bw(calc(pcards.len() as u8))]
    pcards_size: u8,
    #[br(count = pcards_size)]
    pub pcards: Vec<InfoLocation>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 90)]
#[repr(C)]
pub struct Draw {
    pub player: CorePlayer,
    #[bw(calc(codes.len() as u8))]
    codes_size: u8,
    #[br(count = codes_size)]
    #[mask]
    #[mask_if(self.player != player)]
    pub codes: Vec<CardCode>,
}

#[bitfield]
#[derive(BinRead, BinWrite, Debug, Copy, Clone, PartialEq, Eq, Default)]
#[br(map = Self::from_bytes)]
#[bw(map = |&x| Self::into_bytes(x))]
#[repr(u32)]
pub struct CardCode {
    pub id: B28,
    #[skip] __: B3,
    pub is_public: bool,
}

impl GameMessage for CardCode {
    fn mask(&mut self) {
        if !self.is_public() {
            self.set_id(0);
        }
    }
    fn should_mask(&self, _player: CorePlayer) -> bool {
        !self.is_public()
    }
}

impl<T: GameMessage> GameMessage for (T,) {
    fn mask(&mut self) {
        self.0.mask();
    }
    fn should_mask(&self, player: CorePlayer) -> bool {
        self.0.should_mask(player)
    }
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 91)]
#[repr(C)]
pub struct Damage {
    pub player: CorePlayer,
    pub value: i32
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 92)]
#[repr(C)]
pub struct Recover {
    pub player: CorePlayer,
    pub value: i32
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 93)]
#[repr(C)]
pub struct Equip {
    pub position1: InfoLocation,
    pub position2: InfoLocation,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 94)]
#[repr(C)]
pub struct LPUpdate {
    pub player: CorePlayer,
    pub lp: i32
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 95)]
#[repr(C)]
pub struct Unequip {
    pub position: InfoLocation
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 96)]
#[repr(C)]
pub struct CardTarget {
    pub position1: InfoLocation,
    pub position2: InfoLocation,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 97)]
#[repr(C)]
pub struct CancelTarget {
    pub position1: InfoLocation,
    pub position2: InfoLocation,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 100)]
#[repr(C)]
pub struct PayLPCost {
    pub player: CorePlayer,
    pub cost: i32
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 101)]
#[repr(C)]
pub struct AddCounter {
    pub counter_type: u16,
    pub position: CardPosition<false, false, false>,
    pub count: u16
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 102)]
#[repr(C)]
pub struct RemoveCounter {
    pub counter_type: u16,
    pub position: CardPosition<false, false, false>,
    pub count: u16
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 110)]
#[repr(C)]
pub struct Attack {
    pub attacker: InfoLocation,
    pub defenser: InfoLocation,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 111)]
#[repr(C)]
pub struct Battle {
    pub attacker: InfoLocation,
    pub attacker_attack: i32,
    pub attacker_defense: i32,
    #[br(map = |v: u8| v != 0)]
    #[bw(map = |v: &bool| *v as u8)]
    pub attacker_destroyed: bool,
    pub defenser: InfoLocation,
    pub defenser_attack: i32,
    pub defenser_defense: i32,
    #[br(map = |v: u8| v != 0)]
    #[bw(map = |v: &bool| *v as u8)]
    pub defender_destroyed: bool,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 112)]
#[repr(C)]
pub struct AttackDisabled;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 113)]
#[repr(C)]
pub struct DamageStepStart;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 114)]
#[repr(C)]
pub struct DamageStepEnd;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 120)]
#[repr(C)]
pub struct MissedEffect {
    pub location: InfoLocation,
    pub code: i32
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 121)]
#[repr(C)]
pub struct BeChainTarget;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 122)]
#[repr(C)]
pub struct CreateRelation;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 123)]
#[repr(C)]
pub struct ReleaseRelation;

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 130)]
#[repr(C)]
pub struct TossCoin {
    pub player: CorePlayer,
    #[bw(calc(result.len() as u8))]
    result_size: u8,
    #[br(count = result_size)]
    pub result: Vec<i8>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 131)]
#[repr(C)]
pub struct TossDice {
    pub player: CorePlayer,
    #[bw(calc(result.len() as u8))]
    result_size: u8,
    #[br(count = result_size)]
    pub result: Vec<i8>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 132)]
#[repr(C)]
pub struct RockPaperScissors {
    #[wait_for]
    pub player: CorePlayer
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 133)]
#[repr(C)]
pub struct HandResult {
    #[br(temp)]
    #[bw(calc = u8::from(*hand0) | (u8::from(*hand1) << 2))]
    _packed: u8,
    #[br(calc = Hand::try_from_primitive(_packed & 0x03).unwrap())]
    #[bw(ignore)]
    pub hand0: Hand,
    #[br(calc = Hand::try_from_primitive((_packed >> 2) & 0x03).unwrap())]
    #[bw(ignore)]
    pub hand1: Hand,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 140)]
#[repr(C)]
pub struct AnnounceRace {
    #[wait_for]
    pub player: CorePlayer,
    pub announce_count: u8,
    pub available: Race
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 141)]
#[repr(C)]
pub struct AnnounceAttribute {
    #[wait_for]
    pub player: CorePlayer,
    pub announce_count: u8,
    pub available: Attribute
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 142)]
#[repr(C)]
pub struct AnnounceCard {
    #[wait_for]
    pub player: CorePlayer,
    #[bw(calc(value.len() as u8))]
    value_size: u8,
    #[br(count = value_size)]
    pub value: Vec<Operation>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 143)]
#[repr(C)]
pub struct AnnounceNumber {
    #[wait_for]
    pub player: CorePlayer,
    #[bw(calc(value.len() as u8))]
    value_size: u8,
    #[br(count = value_size)]
    pub value: Vec<i32>
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 160)]
#[repr(C)]
pub struct CardHint {
    pub position: InfoLocation,
    pub card_hint_type: crate::constants::CardHint,
    pub value: i32,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 161)]
#[repr(C)]
pub struct TagSwap {
    pub player: CorePlayer,
    pub main_count: u8,
    #[bw(calc(extra_codes.len() as u8))]
    pub extra_count: u8,
    pub extra_p_count: u8,
    #[bw(calc(hand_codes.len() as u8))]
    pub hand_count: u8,
    pub top_code: i32,
    #[br(count = hand_count)]
    #[mask]
    #[mask_if(self.player != player)]
    pub hand_codes: Vec<CardCode>,
    #[br(count = extra_count)]
    #[mask]
    #[mask_if(self.player != player)]
    pub extra_codes: Vec<CardCode>,
}

#[binrw]
#[derive(Debug, Clone, Default)]
pub struct MzoneSlot {
    pub occupied: u8,
    #[br(if(occupied != 0))]
    #[bw(if(*occupied != 0))]
    pub position: u8,
    #[br(if(occupied != 0))]
    #[bw(if(*occupied != 0))]
    pub xyz_count: u8,
}

#[binrw]
#[derive(Debug, Clone)]
pub struct SzonaSlot {
    pub occupied: u8,
    #[br(if(occupied != 0, Position::Any))]
    #[bw(if(*occupied != 0))]
    pub position: Position,
}

#[binrw]
#[derive(Debug, Clone)]
pub struct PlayerField {
    pub lp: i32,
    pub mzone: [MzoneSlot; 7],
    pub szone: [SzonaSlot; 8],
    pub main_count: u8,
    pub hand_count: u8,
    pub grave_count: u8,
    pub remove_count: u8,
    pub extra_count: u8,
    pub extra_p_count: u8,
}

#[binrw]
#[derive(Debug, Clone)]
pub struct ChainLink {
    pub code: u32,
    pub card_position: CardPosition<false, true, false>,
    pub controller: CorePlayer,
    pub location: Location,
    pub sequence: i8,
    pub description: u32,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 162)]
#[repr(C)]
pub struct ReloadField {
    pub duel_rule: MasterRule,
    pub player1: PlayerField,
    pub player2: PlayerField,
    #[bw(calc(chains.len() as u8))]
    pub chain_count: u8,
    #[br(count = chain_count)]
    pub chains: Vec<ChainLink>,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 163)]
pub struct AIName {
    #[bw(calc = name.as_bytes().len() as u16)]
    len: u16,
    #[br(count = len, map = |bytes: Vec<u8>| String::from_utf8(bytes).unwrap())]
    #[bw(map = |s: &String| s.as_bytes().to_vec())]
    #[brw(pad_after = 1)]
    pub name: String,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 164)]
pub struct ShowHint {
    #[bw(calc = name.as_bytes().len() as u16)]
    len: u16,
    #[br(count = len, map = |bytes: Vec<u8>| String::from_utf8(bytes).unwrap())]
    #[bw(map = |s: &String| s.as_bytes().to_vec())]
    #[brw(pad_after = 1)]
    pub name: String,
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 165)]
#[repr(C)]
pub struct PlayerHint {
    pub player: CorePlayer,
    pub player_hint_type: crate::constants::PlayerHint,
    pub value: i32
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 170)]
#[repr(C)]
pub struct MatchKill {
    pub card_code: u32
}

#[binrw]
#[derive(Debug, Clone, Message, GameMessage)]
#[message(gm, flag = 180)]
#[repr(C)]
pub struct CustomMsg {
    #[br(parse_with=until_eof)]
    pub data: Vec<u8>
}

#[cfg(test)]
mod test {
    use binrw::BinRead;
    use binrw::BinWrite;
    use std::io::Cursor;

    #[test]
    fn print_sizes() {
        macro_rules! print_size {
            ($($msg:ident = $flag:literal),* $(,)?) => {
                println!("=== GM ===");
                $(
                    println!("  {:30}: {:>4} bytes", stringify!($msg), std::mem::size_of::<super::$msg>());
                )*
                println!("  {:30}: {:>4} bytes", "MessageType", std::mem::size_of::<super::MessageType>());
                println!("  {:30}: {:>4} bytes", "Message", std::mem::size_of::<super::Message>());
            };
        }
        every_game_message_flat_message!(print_size);
    }

    #[test]
    fn test_ai_name_roundtrip() {
        let original = super::AIName { name: "TestAI".into() };
        let mut writer = Cursor::new(Vec::new());
        original.write_le(&mut writer).unwrap();
        let bytes = writer.into_inner();
        assert_eq!(bytes[0..2], [6, 0], "len should be 6");
        assert_eq!(&bytes[2..8], b"TestAI", "name bytes");
        assert_eq!(bytes[8], 0, "null terminator");
        let roundtripped = super::AIName::read_le(&mut Cursor::new(&bytes)).unwrap();
        assert_eq!(roundtripped.name, "TestAI");
    }
}
