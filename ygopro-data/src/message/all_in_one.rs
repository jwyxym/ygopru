use crate::message::client_to_server;
use crate::message::server_to_client;
use crate::message::game_message;

#[derive(Clone, Copy, Debug)]
pub enum Direction {
    CTOS,
    STOC,
    Other(&'static str)
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Debug, Hash)]
pub enum MessageType {
    STOC(server_to_client::MessageType),
    CTOS(client_to_server::MessageType),
    GM(game_message::MessageType),
    Other(&'static str, u8)
}

#[derive(Debug)]
pub enum Message {
    STOC(server_to_client::Message),
    CTOS(client_to_server::Message),
    Other((&'static str, Box<dyn std::any::Any + Send + Sync>))
}

impl From<server_to_client::MessageType> for MessageType {
    fn from(value: server_to_client::MessageType) -> Self {
        Self::STOC(value)
    }
}
impl From<client_to_server::MessageType> for MessageType {
    fn from(value: client_to_server::MessageType) -> Self {
        Self::CTOS(value)
    }
}

impl From<game_message::MessageType> for MessageType {
    fn from(value: game_message::MessageType) -> Self {
        Self::GM(value)
    }
}

impl From<MessageType> for u8 {
    fn from(value: MessageType) -> Self {
        match value {
            MessageType::STOC(message_type) => message_type.into(),
            MessageType::CTOS(message_type) => message_type.into(),
            MessageType::GM(message_type) => u8::from(message_type),
            MessageType::Other(_, code) => code,
        }
    }
}

#[macro_export]
macro_rules! every_message {
    ($ident: path) => {
        ygopro_data::every_client_to_server_message!($ident);
        ygopro_data::every_server_to_client_message!($ident);
        ygopro_data::every_game_message_message!($ident);
    };
}


