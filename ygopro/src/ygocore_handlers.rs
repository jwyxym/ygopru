
use std::io::Cursor;

use binrw::BinRead;
use linkme::distributed_slice;
use log::warn;

use ygopro_core_wrapper::ProcessResultFlags;
use ygopro_data::constants::*;
use ygopro_data::message::*;
use ygopro_data::message::gm::GameMessage;
use ygopro_handler::Bundle;
use ygopro_handler::FromRequest;
use ygopro_handler::IntoResponse;
use ygopro_derive::before;
use ygopro_derive::handler;
use ygopro_derive::register_to;

use crate::duel;
use crate::duel::Duel;
use crate::player::AllowMessage;
use crate::message as ygopro;
use crate::duel::SendTarget;
use crate::ygopro_handlers;

pub type Request = ygopro_handler::extract::Request<gm::Message, Netplayer>; 
pub type State = ygopro_handlers::State<crate::duel::Duel>;
pub type Handler = ygopro_handler::sync_handler::SyncHandler<Request, State, Response>;

pub struct Response {
    pub target: SendTarget,
    pub refresh: (CorePlayer, Location, i8, Query)
}

impl Default for Response {
    fn default() -> Self {
        Self { target: SendTarget::All, refresh: (CorePlayer::None, Location::empty(), -1, Query::empty()) }
    }
}

impl std::ops::Mul for Response {
    type Output = Response;

    fn mul(self, rhs: Self) -> Self::Output { 
        let target = match (self.target, rhs.target) {
            (SendTarget::All, _) => rhs.target,
            (_, SendTarget::All) => self.target,
            (_, _) => rhs.target
        };
        let refresh = (
            self.refresh.0 * rhs.refresh.0, 
            self.refresh.1 | rhs.refresh.1, 
            std::cmp::max(self.refresh.2, rhs.refresh.2),
            self.refresh.3 | rhs.refresh.3
        );
        Response { target, refresh }
    }
}

impl<'a> std::ops::Not for &'a Response {
    type Output = bool;

    fn not(self) -> bool { false }
}

impl IntoResponse<Response> for () {
    fn into_response(self) -> Response {
        Default::default()
    }
}

impl IntoResponse<Response> for Netplayer {
    fn into_response(self) -> Response {
        Response { target: SendTarget::Single(self), refresh: (CorePlayer::None, Location::empty(), -1, Query::empty()) }
    }
}

impl IntoResponse<Response> for CorePlayer {
    fn into_response(self) -> Response {
        Response { target: SendTarget::Core(self), refresh: (CorePlayer::None, Location::empty(), -1, Query::empty()) }
    } 
}

impl IntoResponse<Response> for SendTarget {
    fn into_response(self) -> Response {
        Response { target: self, refresh: (CorePlayer::None, Location::empty(), -1, Query::empty()) }
    }
}

impl IntoResponse<Response> for (CorePlayer, Location) {
    fn into_response(self) -> Response {
        Response { target: SendTarget::All, refresh: (self.0, self.1, -1, Query::empty()) }
    }
}

impl IntoResponse<Response> for (CorePlayer, Location, i8) {
    fn into_response(self) -> Response {
        Response { target: SendTarget::All, refresh: (self.0, self.1, self.2, Query::empty()) }
    }
}

impl IntoResponse<Response> for (CorePlayer, Location, i8, Query) {
    fn into_response(self) -> Response {
        Response { target: SendTarget::All, refresh: self }
    }
}

#[distributed_slice]
pub static YGOCORE_HANDLERS: [fn() -> (u8, Handler)];

#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

/// process input messages, until waiting for user input or duel end.
/// named `process` in original ygopro.
pub fn evolve(duel: &mut crate::duel::Duel) -> Vec<gm::Message> {
    if duel.ended { return vec![]; }
    let mut messages = vec![];
    loop {
        let result = duel.process();
        let engine_flag = result.flags();
        let engine_length = result.data_length() as usize;
        if engine_length > 0 {
            duel.core.get_message(&mut duel.core_request_buffer[..]);
            let mut cursor = Cursor::new(&duel.core_request_buffer[..engine_length]);
            while let Ok(message) = gm::Message::read_le(&mut cursor) {
                messages.push(message);
            }
        }
        if engine_flag == ProcessResultFlags::End { break; }
        // we should use engine_flag is Flags::Waiting to check if need continue.
        // but sadly, ygocore will incorrectly send waiting even need to continue.
        // so just like original ygopro do, we check specific message here.
        // and also, win is not last message of core. it will repeatedly send win.
        // though the full duel ends, but we still need to break it so that we wouldn't
        // repeatly count the winner.
        if messages.last().map_or(false, |m| m.waiting_for().is_some() || 
                    matches!(gm::MessageType::from(m), gm::MessageType::Retry | gm::MessageType::Win)) { break; }
    }
    messages
}

#[before(ygopro_handler::All)]
#[register_to(YGOCORE_HANDLERS)]
fn on_all_message(message: &gm::Message) -> SendTarget {
    message.waiting_for().map(SendTarget::Core).unwrap_or(SendTarget::All)
}

// #[handler(gm::Retry)]
// #[register_to(YGOCORE_HANDLERS)]
// fn on_retry(duel: &mut SingleDuel, _message: &gm::Retry) -> SendTarget {
//     duel.last_response.map(|player_index| SendTarget::Core(duel.to_core_player(player_index))).unwrap_or(SendTarget::None)
// }

// #[handler(gm::Hint)]
// #[register_to(YGOCORE_HANDLERS)]
// fn on_hint(duel: &mut SingleDuel, message: &gm::Hint) -> SendTarget {
//     match message._type {
//         Hint::Event | Hint::Message | Hint::SelectMessage | Hint::Effect => SendTarget::Core(message.player),
//         Hint::OpponentSelected | Hint::Race | Hint::Attribute | Hint::Code | Hint::Number | Hint::Zone =>
//             SendTarget::Except(duel.to_net_player(message.player)),
//         Hint::Card => SendTarget::All,
//     }
// }

#[handler(gm::Win)]
#[register_to(YGOCORE_HANDLERS)]
fn on_win(duel: &mut Duel, message: &gm::Win) {
    duel.queue_request_ex(ygopro::DuelEnd { winner: message.winner, reason: message.reason });
}

#[handler(gm::SelectBattleCommand)]
#[register_to(YGOCORE_HANDLERS)]
fn on_select_battle_command(_message: &gm::SelectBattleCommand) -> (CorePlayer, Location, i8, Query) {
    (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand, -1, Query::empty())
}

#[handler(gm::SelectIdleCommand)]
#[register_to(YGOCORE_HANDLERS)]
fn on_select_idle_command(_message: &gm::SelectIdleCommand) -> (CorePlayer, Location, i8, Query) {
    (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand, -1, Query::empty())
}

#[handler(gm::ConfirmCards)]
#[register_to(YGOCORE_HANDLERS)]
fn on_confirm_cards(message: &gm::ConfirmCards) -> SendTarget {
    let is_deck = message.cards.first().map_or(false, |c| c.location == Location::Deck);
    if is_deck {
        SendTarget::Core(message.player)
    } else {
        SendTarget::All
    }
}

#[handler(gm::ShuffleHand)]
#[register_to(YGOCORE_HANDLERS)]
fn on_shuffle_hand(message: &gm::ShuffleHand) -> (CorePlayer, Location, i8, Query) {
    (message.player, Location::Hand, -1, Query::from_bits_retain(0x781fff))
}

#[handler(gm::SwapGraveDeck)]
#[register_to(YGOCORE_HANDLERS)]
fn on_swap_grave_deck(message: &gm::SwapGraveDeck) -> (CorePlayer, Location) {
    (message.player, Location::Grave)
}

#[handler(gm::ShuffleSetCard)]
#[register_to(YGOCORE_HANDLERS)]
fn on_shuffle_set_card(message: &gm::ShuffleSetCard) -> (CorePlayer, Location, i8, Query) {
    (CorePlayer::All, message.location, -1, Query::from_bits_retain(0x181fff))
}

#[handler(gm::ShuffleExtra)]
#[register_to(YGOCORE_HANDLERS)]
fn on_shuffle_extra(message: &gm::ShuffleExtra) -> (CorePlayer, Location) {
    (message.player, Location::Extra)
}

#[handler(gm::NewTurn)]
#[register_to(YGOCORE_HANDLERS)]
fn on_new_turn() -> (CorePlayer, Location) {
    // let time_limit = duel.host_info.time_limit;
    // for duel_player in duel.players.iter_mut().flatten() {
    //     duel_player.time_limit = time_limit;
    // }
    (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
}

#[handler(gm::NewPhase)]
#[register_to(YGOCORE_HANDLERS)]
fn on_new_phase() -> (CorePlayer, Location) {
    (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
}

#[handler(gm::Move)]
#[register_to(YGOCORE_HANDLERS)]
fn on_move(message: &gm::Move) -> (CorePlayer, Location, i8) {
    let current_controller = message.current.controller;
    let current_location = message.current.location;
    let current_sequence = message.current.sequence;
    let previous_controller = message.previous.controller;
    let previous_location = message.previous.location;
    if current_location != Location::empty()
        && !current_location.intersects(Location::Overlay)
        && (current_location != previous_location || current_controller != previous_controller)
    {
        (current_controller, current_location, current_sequence as i8)
    } else {
        (CorePlayer::None, Location::empty(), -1)
    }
}

#[handler(gm::PositionChange)]
#[register_to(YGOCORE_HANDLERS)]
fn on_position_change(message: &gm::PositionChange) -> (CorePlayer, Location, i8) {
    if message.previous_position.is_face_down() && !message.current_position.is_face_down() {
        (message.controller, message.location, message.sequence as i8)
    } else {
        (CorePlayer::None, Location::empty(), -1)
    }
}

// #[handler(gm::Swap)]
// #[register_to(YGOCORE_HANDLERS)]
// fn on_swap(duel: &mut SingleDuel, message: &gm::Swap) {
//     let p1 = &message.position1;
//     let p2 = &message.position2;
//     duel.refresh(p1.controller, p1.location, p1.sequence as i8, Query::empty());
//     duel.refresh(p2.controller, p2.location, p2.sequence as i8, Query::empty());
// }

#[handler(gm::Summoned)]
#[register_to(YGOCORE_HANDLERS)]
fn on_summoned() -> (CorePlayer, Location) {
    (CorePlayer::All, Location::MZone | Location::SZone)
}

#[handler(gm::SpecialSummoned)]
#[register_to(YGOCORE_HANDLERS)]
fn on_special_summoned() -> (CorePlayer, Location) {
    (CorePlayer::All, Location::MZone | Location::SZone)
}

#[handler(gm::FlipSummoning)]
#[register_to(YGOCORE_HANDLERS)]
fn on_flip_summoning(message: &gm::FlipSummoning) -> (CorePlayer, Location, i8) {
    let p = &message.position;
    (p.controller, p.location, p.sequence as i8)
}

#[handler(gm::FlipSummoned)]
#[register_to(YGOCORE_HANDLERS)]
fn on_flip_summoned() -> (CorePlayer, Location) {
    (CorePlayer::All, Location::MZone | Location::SZone)
}

#[handler(gm::Chained)]
#[register_to(YGOCORE_HANDLERS)]
fn on_chained() -> (CorePlayer, Location) {
    (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
}

#[handler(gm::ChainSolved)]
#[register_to(YGOCORE_HANDLERS)]
fn on_chain_solved() -> (CorePlayer, Location) {
    (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
}

#[handler(gm::ChainEnd)]
#[register_to(YGOCORE_HANDLERS)]
fn on_chain_end() -> (CorePlayer, Location) {
    (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
}

#[handler(gm::CardSelected)]
#[register_to(YGOCORE_HANDLERS)]
fn on_card_selected() -> SendTarget {
    SendTarget::None
}

#[handler(gm::DamageStepStart)]
#[register_to(YGOCORE_HANDLERS)]
fn on_damage_step_start() -> (CorePlayer, Location) {
    (CorePlayer::All, Location::MZone)
}

#[handler(gm::DamageStepEnd)]
#[register_to(YGOCORE_HANDLERS)]
fn on_damage_step_end() -> (CorePlayer, Location) {
    (CorePlayer::All, Location::MZone)
}

#[handler(gm::MissedEffect)]
#[register_to(YGOCORE_HANDLERS)]
fn on_missed_effect(message: &gm::MissedEffect) -> CorePlayer {
    message.location.controller
}

#[handler(gm::MatchKill)]
#[register_to(YGOCORE_HANDLERS)]
fn on_match_kill(duel: &mut Duel, message: &gm::MatchKill) {
    duel.match_kill_card_code = message.card_code as i32;
}
