//! Allow join duel on middle of duel.
//! 
//! When that happened, server will accept user as observer, and send all history message to that user.
//!
//! # Examples
//!
//! Enable the module:
//!
//! ```
//! let mut configuration = ygopro::Configuration::default();
//! configuration.enable_plugin("ygopro::plugin::soumatou");
//! ```

use log::warn;

use ygopro_data::constants::DuelStage;
use ygopro_data::constants::Netplayer;
use ygopro_data::message::ctos;
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
fn soumatou(duel: &mut Duel, arguments: &[u8; 8]) {
    let target = Netplayer::Observer(arguments[0]).into();
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
        duel.sender.send_without_record(message.clone(), target);
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
            duel.request_sender.send(crate::duel::Request::Command { name: "soumatou", arguments: [index, 0, 0, 0, 0, 0, 0, 0] }).ok();
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
