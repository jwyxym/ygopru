//! Per-duel clock.
//! The time limit / timeout policy lives here as a plugin so it can be
//! swapped without touching duel core logic. `TimerTick` is a command so the
//! heartbeat flows through the normal command channel.

use linkme::distributed_slice;
use log::warn;

use ygopro_data::constants::*;
use ygopro_data::message::ctos;
use ygopro_data::message::gm;
use ygopro_derive::*;

use crate::duel::Duel;
use crate::duel::PlayerIndex;
use crate::duel::Request;
use crate::duel::SendTarget;
use crate::message as ygopro;
use crate::player::AllowMessage;
use crate::single_duel::PlayerTransformer;
use crate::single_duel::SingleDuel;
use crate::single_duel::ygopro_handlers::HandlerEx;
use crate::single_duel::ygopro_handlers::SINGLE_DUEL_YGOPRO_HANDLERS_EX;
use crate::tag_duel::TagDuel;
use crate::tag_duel::ygopro_handlers::HandlerEx as TagDuelHandlerEx;
use crate::tag_duel::ygopro_handlers::TAG_DUEL_YGOPRO_HANDLERS_EX;
use crate::ygocore_handlers::Handler as YgocoreHandler;
use crate::ygocore_handlers::YGOCORE_HANDLERS;
use crate::ygopro_handlers::Handler as YgoproHandler;
use crate::ygopro_handlers::HandlerEx as YgoproHandlerEx;
use crate::ygopro_handlers::YGOPRO_HANDLERS;
use crate::ygopro_handlers::YGOPRO_HANDLERS_EX;

#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

#[derive(Attachment)]
pub struct TimeLimit {
    pub(crate) time_elapsed: u32,
    timer_task: Option<tokio::task::JoinHandle<()>>,
}

impl TimeLimit {
    fn start_timer(&mut self, duel: &Duel) {
        let sender = duel.request_sender.clone();
        self.timer_task = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                if sender.send(Request::Command { name: "timer_tick", arguments: [0; 8] }).is_err() { break; }
            }
        }));
    }
}

#[after(ctos::CreateGame)]
#[register_to(YGOPRO_HANDLERS as YgoproHandler)]
fn on_create_game(duel: &mut Duel, attachment: &mut TimeLimit) {
    duel.last_response = None;
    attachment.time_elapsed = 0;
}

#[after(ygopro::DuelStart)]
#[register_to(YGOPRO_HANDLERS_EX as YgoproHandlerEx)]
fn on_duel_start(duel: &mut Duel, attachment: &mut TimeLimit) {
    let time_limit = duel.host_info.time_limit;
    if time_limit > 0 {
        attachment.time_elapsed = 0;
        for duel_player in duel.players.iter_mut().flatten() {
            duel_player.time_limit = time_limit;
            duel_player.time_compensator = 0;
        }
        attachment.start_timer(duel);
    }
}

#[after(gm::NewTurn)]
#[register_to(YGOCORE_HANDLERS as YgocoreHandler)]
fn on_new_turn(duel: &mut Duel, attachment: &mut TimeLimit) {
    let time_limit = duel.host_info.time_limit;
    for duel_player in duel.players.iter_mut().flatten() {
        duel_player.time_limit = time_limit;
    }
    attachment.time_elapsed = 0;
}

#[command]
#[register_to(crate::command::COMMANDS as crate::command::CommandHandler with &'static str)]
fn timer_tick(duel: &mut Duel, attachment: &mut TimeLimit, _arguments: &[u8; 8]) -> &'static str {
    if let Some(last_response) = duel.last_response && duel.host_info.time_limit > 0 {
        attachment.time_elapsed = attachment.time_elapsed.saturating_add(1);
        let timed_out = duel.get(last_response)
            .map_or(false, |player| attachment.time_elapsed >= player.time_limit as u32);
        if timed_out {
            duel.queue_request_ex(ygopro::Timeout { player: last_response });
        }
    }
    "continue"
}

#[handler(ygopro::Timeout)]
#[register_to(SINGLE_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_timeout(duel: &mut SingleDuel, transformer: PlayerTransformer, timeout: &ygopro::Timeout) {
    let winner = transformer.to_core_player(Netplayer::Player(timeout.player.0)).opponent();
    duel.last_response = None;
    duel.sender.send(gm::Win { winner, reason: WinReason::Timeout }.into(), SendTarget::All);
    duel.queue_request_ex(ygopro::DuelEnd { winner, reason: WinReason::Timeout });
}

#[handler(ygopro::Timeout)]
#[register_to(TAG_DUEL_YGOPRO_HANDLERS_EX as TagDuelHandlerEx)]
fn on_timeout_tag(duel: &mut TagDuel, timeout: &ygopro::Timeout) {
    let winner = duel.player_transformer().team_to_core_player(timeout.player.team().opponent());
    duel.last_response = None;
    duel.sender.send(gm::Win { winner, reason: WinReason::Timeout }.into(), SendTarget::All);
    duel.queue_request_ex(ygopro::DuelEnd { winner, reason: WinReason::Timeout });
}

#[before(ctos::TimeConfirm)]
#[register_to(YGOPRO_HANDLERS as YgoproHandler)]
fn on_time_confirm(duel: &mut Duel, index: PlayerIndex, attachment: &mut TimeLimit) {
    if duel.host_info.time_limit == 0 { return; }
    if Some(index) != duel.last_response {
        warn!("TimeConfirm requested by wrong player");
        return;
    }
    let Some(duel_player) = duel.get_mut(index) else {
        warn!("TimeConfirm requested but player slot is empty");
        return;
    };
    duel_player.state = AllowMessage::Some(ctos::MessageType::Response);
    duel_player.time_limit = duel_player.time_limit.saturating_sub(attachment.time_elapsed as u16);
    attachment.time_elapsed = 0;
}

#[after(ctos::Response)]
#[register_to(YGOPRO_HANDLERS as YgoproHandler)]
fn on_response(duel: &mut Duel, player: PlayerIndex, attachment: &mut TimeLimit) {
    if duel.host_info.time_limit > 0 {
        let time_elapsed = attachment.time_elapsed;
        attachment.time_elapsed = 0;
        if let Some(duel_player) = duel.get_mut(player) {
            duel_player.time_limit = duel_player.time_limit.saturating_sub(time_elapsed as u16);
        }
    }
}

#[after(ygopro::DuelEnd)]
#[register_to(YGOPRO_HANDLERS_EX as YgoproHandlerEx)]
fn on_duel_end(attachment: &mut TimeLimit) {
    if let Some(task) = attachment.timer_task.as_mut() { task.abort(); }
}
