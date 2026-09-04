//! Allow join duel on middle of duel.
//! 
//! When that happened, server will accept user as observer, and send all history message to that user.
//! 
//! # Notes
//!
//! These messages are skipped when replaying the history to an observer:
//! 
//! - `JoinGame`: the observer will receives its own join response.
//! - `SelectHand` / `SelectTp`: directed at a specific duelist to pick a hand or turn.
//! - `TimeLimit`: directed at a duelist's turn timer.
//! - `ChangeSide`: only relevant during siding, which is not replayed.
//! - `HandResult`: the hand result is delivered to the duelists, not observers.
//! - `TeammateSurrender`: specific to a tag-duel team player.
//! - any `waiting_for` game message: an observer must not be prompted to choose.
//!
//! # Examples
//!
//! Enable the module:
//!
//! ```
//! let mut configuration = ygopro::Configuration::default();
//! configuration.enable_plugin("ygopro::plugin::soumatou");
//! ```

use std::any::Any;
use std::ops::Deref;

use log::warn;

use ygopro_data::constants::DuelStage;
use ygopro_data::constants::Netplayer;
use ygopro_data::message::ctos;
use ygopro_data::message::gm::GameMessage;
use ygopro_data::message::stoc;

use ygopro_derive::after;
use ygopro_derive::command;
use ygopro_derive::register_to;
use ygopro_handler::MessageKey;

use crate::duel::Duel;
use crate::message as ygopro;
use crate::ygopro_handlers::Handler;
use crate::ygopro_handlers::HandlerEx;
use crate::ygopro_handlers::Response;
use crate::ygopro_handlers::YGOPRO_HANDLERS;
use crate::ygopro_handlers::YGOPRO_HANDLERS_EX;

/// Name for activitating this module in the plugin system.
pub static NAME: &'static str = module_path!();

#[command]
#[register_to(crate::command::COMMANDS as crate::command::CommandHandler with &'static str)]
fn soumatou(duel: &mut Duel, arguments: &mut Box<dyn Any + Send>) {
    let Some(target) = arguments.downcast_ref::<Netplayer>().copied() else {
        return;
    };
    let duel_start_key = stoc::DuelStart.message_key();
    let directed_keys = [
        stoc::MessageType::JoinGame.message_key(),
        stoc::MessageType::SelectTp.message_key(),
        stoc::MessageType::SelectHand.message_key(),
        stoc::MessageType::TimeLimit.message_key(),
        stoc::MessageType::ChangeSide.message_key(),
        stoc::MessageType::HandResult.message_key(),
        stoc::MessageType::TeammateSurrender.message_key(),
    ];
    for message in duel.sender.masked_messages.iter().skip_while(|message| message.message_key() != duel_start_key) {
        if directed_keys.contains(&message.message_key()) { continue; }
        if let stoc::Message::GameMessage(game_message) = message.deref() && game_message.message.waiting_for().is_some() {
            continue;
        }
        duel.sender.send_without_record(message.clone(), target.into());
    }
}

#[after(ctos::JoinGame)]
#[register_to(YGOPRO_HANDLERS)]
fn on_join(duel: &mut Duel, player: Netplayer) {
    if duel.stage > DuelStage::Begin {
        if let Netplayer::Observer(index) = player {
            // we cannot directly send all messages here becuase
            // we need to wait messages typechange, playerenter, etc
            // which is in current result send to user.
            // so the only way is to make that a command and that
            // will queue after the current response messages.
            duel.request_sender.send(crate::duel::Request::Command { name: "soumatou", arguments: Some(Box::new(Netplayer::Observer(index))) }).ok();
        }
    }
}

#[after(ygopro::ClientJoin)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_client_join(duel: &mut Duel, join: &mut ygopro::ClientJoin, response: &mut Response) {
    if duel.stage > DuelStage::Begin {
        if let Some(oneshot) = join.position_sender.take() {
            *response = Response::Continue;
            let player = crate::player::BaseDuelPlayer::new(join.stoc_sender.clone());
            let undecided_index = duel.uninit_players.insert(player);
            duel.sender.undecided.insert(join.stoc_sender.clone());
            oneshot.send(Netplayer::Undecided(undecided_index as u8)).ok();
        } else {
            warn!("ClientJoin try to send the position, but find it already taken.")
        }
    }
}
