//! Resend the duel history to observers that join after the duel started.
//! Whether late joins are allowed and how history is replayed are deployment
//! decisions, not duel core logic, so they live in a plugin instead of `single_duel`.

use log::warn;

use ygopro_data::constants::DuelStage;
use ygopro_data::constants::Netplayer;
use ygopro_data::message::ctos;

use ygopro_derive::after;
use ygopro_derive::command;
use ygopro_derive::register_to;

use crate::common;
use crate::common::Response;
use crate::message as ygopro;
use crate::single_duel::SingleDuel;
use crate::single_duel::ygopro_handlers::YGOPRO_HANDLERS;
use crate::single_duel::ygopro_handlers::YGOPRO_HANDLERS_EX;
use crate::single_duel::ygopro_handlers::Handler as ygopro_handler;
use crate::single_duel::ygopro_handlers::HandlerEx as ygopro_handler_ex;

pub static NAME: &'static str = module_path!();

#[command]
#[register_to(crate::single_duel::COMMANDS as crate::single_duel::CommandHandler with &'static str)]
fn soumatou(duel: &mut SingleDuel, arguments: &[u8; 8]) {
    let target = Netplayer::Observer(arguments[0]).into();
    for message in &duel.masked_messages {
        duel._send(message.clone(), target);
    }
}

#[after(ctos::JoinGame)]
#[register_to(YGOPRO_HANDLERS as ygopro_handler)]
fn on_join(duel: &mut SingleDuel, player: Netplayer) {
    if duel.stage > DuelStage::Begin {
        if let Netplayer::Observer(index) = player {
            // we cannot directly send all messages here becuase
            // we need to wait messages typechange, playerenter, etc
            // which is in current result send to user.
            // so the only way is to make that a command and that
            // will queue after the current response messages.
            duel.request_sender.send(crate::single_duel::Request::Command { name: "soumatou", arguments: [index, 0, 0, 0, 0, 0, 0, 0] }).ok();
        }
    }
}

#[after(ygopro::ClientJoin)]
#[register_to(YGOPRO_HANDLERS_EX as ygopro_handler_ex)]
fn on_client_join(duel: &mut SingleDuel, join: &mut ygopro::ClientJoin, response: &mut Response) {
    if duel.stage > DuelStage::Begin {
        if let Some(oneshot) = join.position_sender.take() {
            *response = Response::Continue;
            let player = common::DuelPlayer::new(join.stoc_sender.clone());
            let undecided_index = SingleDuel::insert_to_last_available_index(&mut duel.uninit_players, player);
            oneshot.send(Netplayer::Undecided(undecided_index as u8)).ok();
        } else {
            warn!("ClientJoin try to send the position, but find it already taken.")
        }
    }
}
