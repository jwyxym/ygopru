
//! Player abstracts the data stream input into a duel.
//!
//! A player wraps the [`ctos::Message`] receive channel together with the state needed to drive
//! a duel, such as which messages are currently allowed. [`BaseDuelPlayer`] is the raw receiver
//! plus a message filter, and [`DuelPlayer`] adds the per-duel state (deck, time, readiness).

use std::ops::{Deref, DerefMut};

use tokio::sync::mpsc;

use ygopro_data::constants::*;
use ygopro_data::data::*;
use ygopro_data::message::ctos;
use ygopro_data::string::FixedLengthString;

/// Whether a given [`ctos::Message`] is allowed to be passed in by the player.
pub enum AllowMessage {
    None,
    Any,
    Some(ctos::MessageType)
}

impl AllowMessage {
    pub fn allowed(&self, message: &ctos::Message) -> bool {
        let message_type = ctos::MessageType::from(message);
        if matches!(message_type, ctos::MessageType::Chat | ctos::MessageType::Surrender | ctos::MessageType::LeaveGame | ctos::MessageType::RequestField) {
            return true;
        }
        match &self {
            AllowMessage::None => false,
            AllowMessage::Any => true,
            AllowMessage::Some(allowed_type) => *allowed_type == message.into()
        }
    }
}

impl From<AllowMessage> for u8 {
    fn from(value: AllowMessage) -> Self {
        match value {
            AllowMessage::None => 0xff,
            AllowMessage::Any => 0,
            AllowMessage::Some(t) => t.into()
        }
    }
}

impl From<u8> for AllowMessage {
    fn from(value: u8) -> Self {
        match value {
            0xff => AllowMessage::None,
            0 => AllowMessage::Any,
            _ => AllowMessage::Some(ctos::MessageType::from(value))
        }
    }
}

/// The raw player: just a [`ctos::Message`] sender channel plus the current message filter.
pub struct BaseDuelPlayer<Message> {
    pub name: FixedLengthString<20>,
    pub stoc_sender: mpsc::UnboundedSender<Message>,
    pub state: AllowMessage,
}

impl<Message> BaseDuelPlayer<Message> {
    pub fn new(stoc_sender: mpsc::UnboundedSender<Message>) -> Self {
        Self {
            name: FixedLengthString::new(String::new()),
            stoc_sender,
            state: AllowMessage::Any
        }
    }
}

impl<Message> AsMut<BaseDuelPlayer<Message>> for BaseDuelPlayer<Message> {
    fn as_mut(&mut self) -> &mut BaseDuelPlayer<Message> { self }
}

/// A player in a duel, adding the per-duel state on top of [`BaseDuelPlayer`].
pub struct DuelPlayer<Message> {
    pub player: BaseDuelPlayer<Message>,
    pub ready: bool,
    pub deck: Deck,
    pub hand: Option<Hand>,
    pub deck_error: Option<DeckError>,
    pub time_limit: u16,
    pub time_compensator: u16,
    pub time_backed: u16,
}

impl<Message> From<BaseDuelPlayer<Message>> for DuelPlayer<Message> {
    fn from(value: BaseDuelPlayer<Message>) -> Self {
        Self {
            player: value, 
            ready: false,
            deck: Deck::new(),
            hand: None,
            deck_error: None,
            time_limit: 0,
            time_compensator: 0,
            time_backed: 0,
        }
    }
}

impl<Message> Deref for DuelPlayer<Message> {
    type Target = BaseDuelPlayer<Message>;
    fn deref(&self) -> &Self::Target { &self.player }
}

impl<Message> DerefMut for DuelPlayer<Message> {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.player }
}

impl<Message> AsRef<BaseDuelPlayer<Message>> for DuelPlayer<Message> {
    fn as_ref(&self) -> &BaseDuelPlayer<Message> { &self.player }
}

impl<Message> AsMut<BaseDuelPlayer<Message>> for DuelPlayer<Message> {
    fn as_mut(&mut self) -> &mut BaseDuelPlayer<Message> { &mut self.player }
}

// impl<Response> FromRequest<common::Request, State<SingleDuel>, Response> for &mut DuelPlayer where Request: Send + Sync, Response: Send {
//     fn from_request(bundle: &mut Bundle<common::Request, State<SingleDuel>, Response>) -> Option<Self> {
//         let player = bundle.state.duel.get_player_mut(bundle.request.extra)?;
//         Some(unsafe { &mut *(player as *mut DuelPlayer) })
//     }
// }
