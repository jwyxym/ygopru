use log::warn;
use tokio::sync::mpsc;

use ygopro_data::complex::Complex;
use ygopro_data::constants::*;
use ygopro_data::message;
use ygopro_derive::Message;

use crate::single_duel::SingleDuel;
use crate::common::SendTarget;


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

#[derive(Debug, Message)]
#[message(ygopro, flag = 2)]
pub struct FirstShuffle;

#[derive(Debug, Message)]
#[message(ygopro, flag = 3)]
pub struct DuelInit;

#[derive(Debug, Message)]
#[message(ygopro, flag = 4)]
pub struct DuelStart;

#[derive(Debug, Message)]
#[message(ygopro, flag = 10)]
pub struct DuelEnd {
    pub winner: CorePlayer,
    pub reason: WinReason
}

#[derive(Debug, Message)]
#[message(ygopro, flag = 11)]
pub struct GenerateReplay;

#[derive(Debug, Message)]
#[message(ygopro, flag = 12)]
pub struct JudgeContinueMatch;

#[derive(Debug, Message)]
#[message(ygopro, flag = 13)]
pub struct RecreateDuel;

#[derive(Debug, Message)]
#[message(ygopro, flag = 20)]
pub struct MatchEnd;

#[derive(Debug, Message)]
#[message(ygopro, flag = 255)]
pub struct Terminate;

trait Next {
    fn process_continue(_duel: &mut SingleDuel) {}
    fn process_terminate(duel: &mut SingleDuel) {
        duel.send_request_ex(MatchEnd);
    }
}

impl Next for ClientJoin {}
impl Next for FirstShuffle {}
impl Next for DuelInit {
    fn process_continue(duel: &mut SingleDuel) {
        duel.send_request_ex(DuelStart);
    }
}
impl Next for DuelStart {}

impl Next for DuelEnd {
    fn process_continue(duel: &mut SingleDuel) {
        duel.send_request_ex(GenerateReplay);
        duel.send_request_ex(JudgeContinueMatch);
    }
}

impl Next for GenerateReplay {}

impl Next for JudgeContinueMatch {
    fn process_continue(duel: &mut SingleDuel) {
        duel.send_request_ex(RecreateDuel);
    }
}

impl Next for RecreateDuel {}
impl Next for MatchEnd {
    fn process_terminate(_duel: &mut SingleDuel) {
        warn!("Send terminate response in match end processing")
    }
}
impl Next for Terminate {
    fn process_terminate(_duel: &mut SingleDuel) {
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
            pub fn process_continue(self, duel: &mut SingleDuel) {
                match self {
                    $(Message::$message_name(_) => <$message_name as Next>::process_continue(duel)),*
                }
            }

            pub fn process_terminate(self, duel: &mut SingleDuel) {
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

            impl From<$message_name> for crate::common::RequestEx {
                fn from(message: $message_name) -> Self {
                    crate::common::RequestEx { message: message.into(), extra: SendTarget::All }
                }
            }

            impl From<$message_name> for crate::single_duel::Request {
                fn from(message: $message_name) -> Self {
                    crate::single_duel::Request::MessageEx(message.into())
                }
            }

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
    Terminate = 255
);
