//! Internal messages (MessageEx) exchanged inside the duel actor.
//!
//! These hook into and alter parts of ygopro's behavior without occupying the message flags of the
//! ygopro protocol. They are only exchanged inside the duel actor and are never sent over the network.
//! 
//! It can be understand as extra `stoc::Message`.
//!
//! The duel start is driven by this chain:
//!
//! ```text
//! TpResult -> FirstShuffle
//!          -> DuelInit -> DuelStart -> Evolve
//! ```
//!
//! The duel end is driven by this chain:
//!
//! ```text
//! DuelEnd -> GenerateReplay
//!         -> JudgeContinueMatch -> RecreateDuel   (match continues)
//!                               -> MatchEnd       (match ends)
//! ```

use log::warn;
use tokio::sync::mpsc;

use ygopro_data::complex::Complex;
use ygopro_data::constants::*;
use ygopro_data::message;
use ygopro_derive::Message;

use crate::duel::Duel;
use crate::duel::SendTarget;


/// Attach a new client to the room.
///
/// Sent when a new client connects.
#[derive(Debug, Message)]
#[message(ygopro, flag = 1)]
pub struct ClientJoin {
    pub stoc_sender: mpsc::UnboundedSender<Complex<message::stoc::Message>>, 
    pub position_sender: Option<tokio::sync::oneshot::Sender<Netplayer>>
}

impl<Extra, State, Res> ygopro_handler::FromRequest<ygopro_handler::extract::Request<Message, Extra>, State, Res> for &mut ClientJoin
where Extra: Send, State: Send, Res: Send, 
{
    fn from_request(bundle: &mut ygopro_handler::Bundle<ygopro_handler::extract::Request<Message, Extra>, State, Res>) -> Option<Self> {
        if let Message::ClientJoin(inner) = &mut bundle.request.message {
            Some(unsafe { &mut *(inner as *mut ClientJoin) })
        } else {
            None
        }
    }
}

/// Shuffle both players' decks in first-attack order.
///
/// Sent when the duel starts, right after `TpResult`.
#[derive(Debug, Message)]
#[message(ygopro, flag = 2)]
pub struct FirstShuffle;

/// Load both decks into the core with new_card.
///
/// Sent when both decks are ready, before the duel starts.
#[derive(Debug, Message)]
#[message(ygopro, flag = 3)]
pub struct DuelInit;

/// Send init field info (deck + extra), then evolve the ygocore.
///
/// Sent after both decks are loaded into the core.
#[derive(Debug, Message)]
#[message(ygopro, flag = 4)]
pub struct DuelStart;

/// Signal the end of a duel.
///
/// Sent when a duel is ended.
#[derive(Debug, Message)]
#[message(ygopro, flag = 10)]
pub struct DuelEnd {
    pub winner: CorePlayer,
    pub reason: WinReason
}

/// Generate and send the replay.
///
/// Sent when duel is about to end.
#[derive(Debug, Message)]
#[message(ygopro, flag = 11)]
pub struct GenerateReplay;

/// Decide whether the match should continue. Continue leads to [`RecreateDuel`], terminate to [`MatchEnd`].
/// 
/// Sent when duel ends.
#[derive(Debug, Message)]
#[message(ygopro, flag = 12)]
pub struct JudgeContinueMatch;

/// Enter siding, reset player states and recreate the ygocore duel.
///
/// Sent when the match should continue.
#[derive(Debug, Message)]
#[message(ygopro, flag = 13)]
pub struct RecreateDuel;

/// Signal the end of a match.
///
/// Sent when a match is ended.
#[derive(Debug, Message)]
#[message(ygopro, flag = 20)]
pub struct MatchEnd;

/// Report a player's timeout.
///
/// Sent when a player's time runs out.
#[derive(Debug, Message)]
#[message(ygopro, flag = 21)]
pub struct Timeout {
    pub player: crate::duel::PlayerIndex,
}

/// Terminate the room.
///
/// Sent when the room is dropped.
#[derive(Debug, Message)]
#[message(ygopro, flag = 255)]
pub struct Terminate;

trait Next {
    fn process_continue(_duel: &mut Duel) {}
    fn process_terminate(duel: &mut Duel) {
        duel.queue_request_ex(MatchEnd);
    }
}

impl Next for ClientJoin {}
impl Next for FirstShuffle {}
impl Next for DuelInit {
    fn process_continue(duel: &mut Duel) {
        duel.queue_request_ex(DuelStart);
    }
}
impl Next for DuelStart {}

impl Next for DuelEnd {
    fn process_continue(duel: &mut Duel) {
        duel.queue_request_ex(GenerateReplay);
        duel.queue_request_ex(JudgeContinueMatch);
    }
}

impl Next for GenerateReplay {}

impl Next for JudgeContinueMatch {
    fn process_continue(duel: &mut Duel) {
        duel.queue_request_ex(RecreateDuel);
    }
}

impl Next for RecreateDuel {}
impl Next for Timeout {}
impl Next for MatchEnd {
    fn process_terminate(duel: &mut Duel) {
        duel.request_sender.send(crate::duel::Request::Command { name: "terminate", arguments: Some(Box::new([0; 8])) }).ok();
    }
}
impl Next for Terminate {
    fn process_terminate(_duel: &mut Duel) {
        warn!("Send terminate response in terminate processing")
    }
}

macro_rules! generate_enum {
    ($($message_name:ident = $message_flag:literal),*) => {
        #[derive(Debug)]
        pub enum Message {
            $($message_name($message_name)),*
        }

        impl ygopro_data::message::PureMessage for Message {}

        impl Message {
            pub fn process_continue(self, duel: &mut Duel) {
                match self {
                    $(Message::$message_name(_) => <$message_name as Next>::process_continue(duel)),*
                }
            }

            pub fn process_terminate(self, duel: &mut Duel) {
                match self {
                    $(Message::$message_name(_) => <$message_name as Next>::process_terminate(duel)),*
                }
            }
        }

        impl ygopro_handler::MessageKey<u8> for Message {
            fn message_key(&self) -> u8 {
                match self {
                    $(Message::$message_name(_) => $message_flag),*
                }
            }
        }

        $(
            impl From<$message_name> for Message {
                fn from(value: $message_name) -> Self {
                    Message::$message_name(value)
                }
            }

            impl From<$message_name> for crate::ygopro_handlers::RequestEx {
                fn from(message: $message_name) -> Self {
                    crate::ygopro_handlers::RequestEx { message: message.into(), extra: SendTarget::All }
                }
            }

            // impl From<$message_name> for crate::ygopro_handlers::Request {
            //     fn from(message: $message_name) -> Self {
            //         crate::duel::Request::MessageEx(message.into())
            //     }
            // }

            impl TryFrom<Message> for $message_name {
                type Error = ygopro_data::message::Error;

                fn try_from(value: Message) -> Result<Self, Self::Error> {
                    match value {
                        Message::$message_name(value) => Ok(value),
                        _ => Err(ygopro_data::message::Error::WrongType)
                    }
                }
            }

            impl $message_name {
                pub fn into_message(self) -> Message {
                    self.into()
                }
            }

            impl<Extra, State, Res> ygopro_handler::FromRequest<ygopro_handler::extract::Request<Message, Extra>, State, Res> for &$message_name
            where
                Extra: Send,
                State: Send,
                Res: Send,
            {
                fn from_request(bundle: &mut ygopro_handler::Bundle<ygopro_handler::extract::Request<Message, Extra>, State, Res>) -> Option<Self> {
                    if let Message::$message_name(inner) = &bundle.request.message {
                        Some(unsafe { &*(inner as *const $message_name) })
                    } else {
                        None
                    }
                }
            }
        )*
    };
}

generate_enum!(
    ClientJoin = 1,
    FirstShuffle = 2,
    DuelInit = 3,
    DuelStart = 4,
    DuelEnd = 10,
    GenerateReplay = 11,
    JudgeContinueMatch = 12,
    RecreateDuel = 13,
    MatchEnd = 20,
    Timeout = 21,
    Terminate = 255
);
