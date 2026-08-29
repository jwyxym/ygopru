use std::io::Cursor;

use binrw::BinRead;
use linkme::distributed_slice;

use ygopro_data::message::gm::GameMessage;
use ygopro_derive::*;
use ygopro_data::constants::*;
use ygopro_data::data::QueryData;
use ygopro_data::data::UpdateCardInfo;
use ygopro_data::message::{ctos, stoc, gm};

use crate::message as ygopro;
use crate::plugin::time_limit::TimeLimit;
use crate::duel::PlayerIndex;
use crate::single_duel::SingleDuel;
use crate::single_duel::PlayerTransformer as SinglePlayerTransformer;
use crate::single_duel::ygopro_handlers::SINGLE_DUEL_YGOPRO_HANDLERS;
use crate::single_duel::ygopro_handlers::SINGLE_DUEL_YGOPRO_HANDLERS_EX;
use crate::single_duel::ygopro_handlers::Handler as SingleDuelYgoproHandler;
use crate::single_duel::ygopro_handlers::HandlerEx;
use crate::tag_duel::TagDuel;
use crate::tag_duel::PlayerTransformer as TagPlayerTransformer;
use crate::tag_duel::ygopro_handlers::TAG_DUEL_YGOPRO_HANDLERS;
use crate::tag_duel::ygopro_handlers::Handler as TagDuelYgoproHandler;
use crate::ygocore_handlers::YGOCORE_HANDLERS;
use crate::ygocore_handlers::Handler as YgocoreHandler;

#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

#[derive(Attachment)]
pub struct Attachment {
    #[attachment(default = "Phase::Draw")]
    phase: Phase,
    deck_reversed: bool,
    #[attachment(default = "CorePlayer::None")]
    turn_player: CorePlayer
}

#[after(gm::NewPhase)]
#[register_to(YGOCORE_HANDLERS as YgocoreHandler)]
fn on_new_phase(attachment: &mut Attachment, message: &gm::NewPhase) {
    attachment.phase = message.phase;
}

#[after(gm::NewTurn)]
#[register_to(YGOCORE_HANDLERS as YgocoreHandler)]
fn on_new_turn(attachment: &mut Attachment, message: &gm::NewTurn) {
    attachment.turn_player = message.player;
}

#[after(gm::ReverseDeck)]
#[register_to(YGOCORE_HANDLERS as YgocoreHandler)]
fn on_reverse_deck(attachment: &mut Attachment) {
    attachment.deck_reversed = !attachment.deck_reversed;
}

#[after(ygopro::RecreateDuel)]
#[register_to(SINGLE_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_recreate_duel(attachment: &mut Attachment) {
    attachment.phase = Phase::Standby;
    attachment.turn_player = CorePlayer::FirstAttackPlayer;
    attachment.deck_reversed = false;
}

#[handler(ctos::RequestField)]
#[register_to(SINGLE_DUEL_YGOPRO_HANDLERS as SingleDuelYgoproHandler)]
fn on_request_field_single(duel: &mut SingleDuel, transformer: SinglePlayerTransformer, states: &mut anymap3::Map<dyn std::any::Any + Send>, player: PlayerIndex) -> Vec<stoc::Message> {
    let attachment = match states.get::<Attachment>() {
        Some(attachment) => attachment,
        None => return vec![],
    };

    let mut messages = vec![];
    messages.push(stoc::DuelStart.into());

    let core_player = transformer.to_core_player(player.into());
    let player_type = core_player as u8;
    let start_lp = duel.host_info.start_lp as i32;
    messages.push(gm::Start {
        player_type,
        rule: duel.host_info.duel_rule,
        player1_lp: start_lp,
        player2_lp: start_lp,
        player1_deck_count: 0,
        player1_extra_count: 0,
        player2_deck_count: 0,
        player2_extra_count: 0,
    }.into());

    messages.push(gm::NewTurn { player: CorePlayer::FirstAttackPlayer }.into());
    if attachment.turn_player == CorePlayer::SecondAttackPlayer {
        messages.push(gm::NewTurn { player: CorePlayer::SecondAttackPlayer }.into());
    }

    messages.push(gm::NewPhase {
        phase: attachment.phase,
    }.into());

    let len = duel.duel.core.query_field_info(&mut duel.duel.core_request_buffer);
    let mut cursor = Cursor::new(&duel.duel.core_request_buffer[..len as usize]);
    let message = gm::Message::read_le(&mut cursor).unwrap();
    messages.push(message.into());

    let opponent = core_player.opponent();
    for location in [Location::MZone, Location::SZone, Location::Hand, Location::Grave, Location::Extra, Location::Removed] {
        for mut gm_message in duel.refresh_location(opponent, location, Query::all()) {
            if !duel.configuration.no_mask { gm_message.mask(); }
            messages.push(gm_message.into());
        }
        for gm_message in duel.refresh_location(core_player, location, Query::all()) {
            messages.push(gm_message.into());
        }
    }

    if attachment.deck_reversed {
        messages.push(gm::ReverseDeck.into());
    }

    for player in [CorePlayer::FirstAttackPlayer, CorePlayer::SecondAttackPlayer] {
        let message = duel.query_location_cards(player, Location::Deck, Query::Code | Query::Position);
        let message = match message { gm::Message::UpdateData(update) => update, _ => continue };
        let data = match message.data.last() { Some(UpdateCardInfo::Data(data)) => data, _ => continue };
        let code = match &data[0] { QueryData::Code(code) => *code as u32, _ => continue };
        let position = match &data[1] { QueryData::Position(location) => location.position, _ => continue };
        let is_faceup = !position.is_face_down();
        if !attachment.deck_reversed && !is_faceup { continue; }
        messages.push(gm::DeckTop {
            player,
            sequence: 0,
            code: gm::CardCode::new().with_id(code).with_is_public(is_faceup),
        }.into());
    }

    if let Some(time_limit) = states.get::<TimeLimit>() {
        for player_index in [PlayerIndex::Player1, PlayerIndex::Player2] {
            let base = duel.get(player_index).map_or(0, |player| player.time_limit);
            let left_time = if Some(player_index) == duel.last_response {
                base.saturating_sub(time_limit.time_elapsed as u16)
            } else {
                base
            };
            messages.push(stoc::TimeLimit {
                player: transformer.to_core_player(player_index.into()),
                left_time,
            }.into());
        }
    }

    messages.push(stoc::FieldFinish.into());
    if let Some(message) = &duel.last_select_message && duel.last_response.unwrap_or(transformer.to_player_index(CorePlayer::FirstAttackPlayer).unwrap_or(PlayerIndex::Player1)) == player {
        messages.push(message.clone().into());
    }
    messages
}

#[handler(ctos::RequestField)]
#[register_to(TAG_DUEL_YGOPRO_HANDLERS as TagDuelYgoproHandler)]
fn on_request_field_tag(duel: &mut TagDuel, transformer: TagPlayerTransformer, states: &mut anymap3::Map<dyn std::any::Any + Send>, player: PlayerIndex) -> Vec<stoc::Message> {
    let attachment = match states.get::<Attachment>() {
        Some(attachment) => attachment,
        None => return vec![],
    };

    let mut messages = vec![];
    messages.push(stoc::DuelStart.into());

    let core_player = transformer.to_core_player(player.into());
    let player_type = core_player as u8;
    let start_lp = duel.host_info.start_lp as i32;
    messages.push(gm::Start {
        player_type,
        rule: duel.host_info.duel_rule,
        player1_lp: start_lp,
        player2_lp: start_lp,
        player1_deck_count: 0,
        player1_extra_count: 0,
        player2_deck_count: 0,
        player2_extra_count: 0,
    }.into());

    messages.push(gm::NewTurn { player: CorePlayer::FirstAttackPlayer }.into());
    if attachment.turn_player == CorePlayer::SecondAttackPlayer {
        messages.push(gm::NewTurn { player: CorePlayer::SecondAttackPlayer }.into());
    }

    messages.push(gm::NewPhase {
        phase: attachment.phase,
    }.into());

    let len = duel.duel.core.query_field_info(&mut duel.duel.core_request_buffer);
    let mut cursor = Cursor::new(&duel.duel.core_request_buffer[..len as usize]);
    let message = gm::Message::read_le(&mut cursor).unwrap();
    messages.push(message.into());

    let opponent = core_player.opponent();
    for location in [Location::MZone, Location::SZone, Location::Hand, Location::Grave, Location::Extra, Location::Removed] {
        for mut gm_message in duel.refresh_location(opponent, location, Query::all()) {
            if !duel.configuration.no_mask { gm_message.mask(); }
            messages.push(gm_message.into());
        }
        for gm_message in duel.refresh_location(core_player, location, Query::all()) {
            messages.push(gm_message.into());
        }
    }

    if attachment.deck_reversed {
        messages.push(gm::ReverseDeck.into());
    }

    for player in [CorePlayer::FirstAttackPlayer, CorePlayer::SecondAttackPlayer] {
        let message = duel.query_location_cards(player, Location::Deck, Query::Code | Query::Position);
        let message = match message { gm::Message::UpdateData(update) => update, _ => continue };
        let data = match message.data.last() { Some(UpdateCardInfo::Data(data)) => data, _ => continue };
        let code = match &data[0] { QueryData::Code(code) => *code as u32, _ => continue };
        let position = match &data[1] { QueryData::Position(location) => location.position, _ => continue };
        let is_faceup = !position.is_face_down();
        if !attachment.deck_reversed && !is_faceup { continue; }
        messages.push(gm::DeckTop {
            player,
            sequence: 0,
            code: gm::CardCode::new().with_id(code).with_is_public(is_faceup),
        }.into());
    }

    if let Some(time_limit) = states.get::<TimeLimit>() {
        for player_index in [PlayerIndex::Player1, PlayerIndex::Player2, PlayerIndex::Player3, PlayerIndex::Player4] {
            let base = duel.get(player_index).map_or(0, |player| player.time_limit);
            let left_time = if Some(player_index) == duel.last_response {
                base.saturating_sub(time_limit.time_elapsed as u16)
            } else {
                base
            };
            messages.push(stoc::TimeLimit {
                player: transformer.to_core_player(player_index.into()),
                left_time,
            }.into());
        }
    }

    messages.push(stoc::FieldFinish.into());
    if let Some(message) = &duel.last_select_message && duel.last_response.unwrap_or(transformer.to_player_index(CorePlayer::FirstAttackPlayer).unwrap_or(PlayerIndex::Player1)) == player {
        messages.push(message.clone().into());
    }
    messages
}
