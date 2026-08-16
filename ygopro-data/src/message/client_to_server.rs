#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]

use binrw::BinRead;
use binrw::BinWrite;
use ygopro_derive::Message;

use crate::generate_enum;
use crate::constants::CorePlayer;
use crate::data::Deck;
use crate::data::Response as DataResponse;
use crate::message::HostInfo;
use crate::utils::string::{FixedLengthString, U16String};


include!(concat!(env!("OUT_DIR"), "/client_to_server.rs"));
every_client_to_server_flat_message!(generate_enum);

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 1)]
pub struct Response {
    #[br(args_raw = None)]
    pub response: DataResponse
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 2)]
#[repr(C)]
pub struct UpdateDeck {
    pub deck: Deck
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 3)]
#[repr(C)]
pub struct HandResult {
    pub res: crate::constants::Hand
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 4)]
#[repr(C)]
pub struct TpResult {
    #[br(map = |v: CorePlayer| v.opponent())]
    #[bw(map = |v: &CorePlayer| v.opponent())]
    pub result: CorePlayer
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 16)]
#[repr(C)]
pub struct PlayerInfo {
    pub name: FixedLengthString<20>
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 17)]
#[repr(C)]
pub struct CreateGame {
    pub host_info: HostInfo,
    pub name: FixedLengthString<20>,
    pub pass: FixedLengthString<20>
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 18)]
#[repr(C)]
pub struct JoinGame {
    #[brw(pad_after=2)]
    pub version: u16,
    pub gameid: u32,
    pub pass: FixedLengthString<20>
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 19)]
#[repr(C)]
pub struct LeaveGame;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 20)]
#[repr(C)]
pub struct Surrender;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 21)]
#[repr(C)]
pub struct TimeConfirm;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 22)]
#[repr(C)]
pub struct Chat {
    pub msg: U16String
}

impl From<String> for Chat {
    fn from(value: String) -> Self {
        Self { msg: value.into() }
    }
}

impl<'s> From<&'s str> for Chat {
    fn from(value: &'s str) -> Self {
        Self { msg: value.into() }
    }
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 32)]
#[repr(C)]
pub struct HsToDuelist;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 33)]
#[repr(C)]
pub struct HsToObserver;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 34)]
#[repr(C)]
pub struct HsReady;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 35)]
#[repr(C)]
pub struct HsNotReady;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 36)]
#[repr(C)]
pub struct HsKick {
    pub pos: crate::constants::Netplayer
}

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 37)]
#[repr(C)]
pub struct HsStart;

#[derive(BinRead, BinWrite, Debug, Clone, Message)]
#[message(ctos, flag = 48)]
#[repr(C)]
pub struct RequestField;

#[cfg(test)]
mod test {
    #[test]
    fn print_sizes() {
        macro_rules! print_size {
            ($($msg:ident = $flag:literal),* $(,)?) => {
                println!("=== CTOS ===");
                $(
                    println!("  {:30}: {:>4} bytes", stringify!($msg), std::mem::size_of::<super::$msg>());
                )*
                println!("  {:30}: {:>4} bytes", "MessageType", std::mem::size_of::<super::MessageType>());
                println!("  {:30}: {:>4} bytes", "Message", std::mem::size_of::<super::Message>());
            };
        }
        every_client_to_server_flat_message!(print_size);
    }
}

