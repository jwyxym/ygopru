#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]

use binrw::BinRead;
use binrw::BinWrite;
use num_enum::FromPrimitive;
use num_enum::TryFromPrimitive;
use ygopro_derive::Message;

use crate::constants;
use crate::constants::Color;
use crate::constants::CorePlayer;
use crate::constants::Netplayer;
use crate::constants::PlayerChange;
use crate::generate_enum;
use crate::message::game_message;
use crate::utils::string::FixedLengthString;
use crate::utils::string::U16String;


use super::HostInfo;

include!(concat!(env!("OUT_DIR"), "/server_to_client.rs"));
every_server_to_client_flat_message!(generate_enum);

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 1)]
pub struct GameMessage {
    pub message: game_message::Message
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 2)]
#[repr(C)]
pub struct ErrorMessage {
    pub err: crate::constants::ErrorMessage
}

impl From<crate::constants::ErrorMessage> for ErrorMessage {
    fn from(value: crate::constants::ErrorMessage) -> Self {
        Self { err: value }
    }
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 3)]
#[repr(C)]
pub struct SelectHand;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 4)]
#[repr(C)]
pub struct SelectTp;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 5)]
#[repr(C)]
pub struct HandResult {
    pub hand1: crate::constants::Hand,
    pub hand2: crate::constants::Hand
}

impl HandResult {
    pub fn swap(&mut self) {
        let r = self.hand1;
        self.hand1 = self.hand2;
        self.hand2 = r;
    }

    pub fn swap_clone(&self) -> Self {
        let mut cloned = self.clone();
        cloned.swap();
        cloned
    } 

    pub fn judge(&self) -> crate::constants::HandResult {
        self.hand1.judge(&self.hand2)
    }
}

/// reserved, never sent by server
#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 6)]
#[repr(C)]
pub struct TpResult {
    pub result: u8
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 7)]
#[repr(C)]
pub struct ChangeSide;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 8)]
#[repr(C)]
pub struct WaitingSide;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 9)]
#[repr(C)]
pub struct DeckCount {
    pub mainc_s: u16,
    pub extrac_s: u16,
    pub sidec_s: u16,
    pub mainc_o: u16,
    pub extrac_o: u16,
    pub sidec_o: u16
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 17)]
#[repr(C)]
pub struct CreateGame {
    pub gameid: u32
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 18)]
#[repr(C)]
pub struct JoinGame {
    pub info: HostInfo
}

// In rust ygopro, due to actor model, we must rememeber the observer index
// which dont exist in C++ version. Netplayer is a enum which contains necessary
// message, but will lost observer index when into bytes.
// So we set it different with other here. stoc::TypeChange keep all message.
// but constants::TypeChange will discard the observer index.
#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[br(map = |v: constants::TypeChange| v.into() )]
#[bw(map = |v| constants::TypeChange::from(v))]
#[message(stoc, flag = 19)]
#[repr(C)]
pub struct TypeChange {
    pub player: Netplayer,
    pub host: bool,
}

impl From<&TypeChange> for constants::TypeChange {
    fn from(value: &TypeChange) -> Self {
        Self::new().with_host(value.host).with_player(value.player)
    }
}

impl From<constants::TypeChange> for TypeChange {
    fn from(value: constants::TypeChange) -> Self {
        Self {
            player: value.player(),
            host: value.host()
        }
    }
}
 
#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 20)]
#[repr(C)]
pub struct LeaveGame {
    pub pos: Netplayer
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 21)]
#[repr(C)]
pub struct DuelStart;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 22)]
#[repr(C)]
pub struct DuelEnd;

// Rust enum use its variant's max size as its size.
// As we decide to use clone for message dispatching,
// The replay size is too large to be put in the enum, so we box it.
#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 23)]
pub struct Replay {
    pub replay: Box<crate::data::Replay>
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 24)]
#[repr(C)]
pub struct TimeLimit {
    #[brw(pad_after = 1)]
    pub player: CorePlayer,
    pub left_time: u16
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 25)]
#[repr(C)]
pub struct Chat {
    #[brw(pad_after = 1)]
    pub player: ChatSource,
    pub msg: U16String
}

#[derive(BinRead, BinWrite, Copy, Clone, Eq, PartialEq, Debug)]
#[br(try_map = |raw: u8| ChatSource::try_from(raw))]
#[bw(map = |value: &ChatSource| u8::from(*value))]
pub enum ChatSource {
    Player(Netplayer),
    System(Color)
}

impl From<Color> for ChatSource {
    fn from(color: Color) -> Self {
        ChatSource::System(color)
    }
}

impl From<Netplayer> for ChatSource {
    fn from(player: Netplayer) -> Self {
        ChatSource::Player(player)
    }
}

impl TryFrom<ChatSource> for Netplayer {
    type Error = ();

    fn try_from(value: ChatSource) -> Result<Self, Self::Error> {
        match value {
            ChatSource::Player(player) => Ok(player),
            _ => Err(())
        }
    }
}

impl TryFrom<u8> for ChatSource {
    type Error = binrw::Error;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value <= 7 {
            Ok(ChatSource::Player(Netplayer::from_primitive(value)))
        } else {
            Color::try_from_primitive(value)
                .map(ChatSource::System)
                .map_err(|_| binrw::Error::NoVariantMatch { pos: 0 })
        }
    }
}

impl From<ChatSource> for u8 {
    fn from(value: ChatSource) -> u8 {
        match value {
            ChatSource::Player(player) => player.into(),
            ChatSource::System(color) => color.into(),
        }
    }
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 32)]
#[repr(C)]
pub struct HsPlayerEnter {
    pub name: FixedLengthString<20>,
    pub pos: Netplayer
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 33)]
#[repr(C)]
pub struct HsPlayerChange {
    pub status: PlayerChange
}

impl From<PlayerChange> for HsPlayerChange {
    fn from(value: PlayerChange) -> Self {
        HsPlayerChange { status: value }
    }
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 34)]
#[repr(C)]
pub struct HsWatchChange {
    pub watch_count: u16
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 35)]
#[repr(C)]
pub struct TeammateSurrender;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(stoc, flag = 48)]
#[repr(C)]
pub struct FieldFinish;

#[cfg(test)]
mod test {
    #[test]
    fn print_sizes() {
        macro_rules! print_size {
            ($($msg:ident = $flag:literal),* $(,)?) => {
                println!("=== STOC ===");
                $(
                    println!("  {:30}: {:>4} bytes", stringify!($msg), std::mem::size_of::<super::$msg>());
                )*
                println!("  {:30}: {:>4} bytes", "MessageType", std::mem::size_of::<super::MessageType>());
                println!("  {:30}: {:>4} bytes", "Message", std::mem::size_of::<super::Message>());
            };
        }
        every_server_to_client_flat_message!(print_size);
    }
}
