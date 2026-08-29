//! Time compensator buffer.
//! Small delays are taken from a compensator instead of the real time limit,
//! and meaningful operations refill it. This policy is a deployment decision,
//! not duel core logic, so it lives in a plugin instead of `single_duel`.

use linkme::distributed_slice;

use ygopro_data::message::ctos;
use ygopro_derive::Configuration;
use ygopro_derive::after;
use ygopro_derive::before;
use ygopro_derive::register_to;

use crate::duel::Duel;
use crate::duel::PlayerIndex;
use crate::duel::response_is_meaningful;
use crate::plugin::time_limit::TimeLimit;
use crate::ygopro_handlers::Handler as ygopro_handler;
use crate::ygopro_handlers::YGOPRO_HANDLERS;

#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

#[derive(Clone, Configuration)]
pub struct Configuration {
    #[config(default = "1")]
    pub add_small_time_deposit_after_operation: u16,
    #[config(default = "10")]
    pub ignore_small_time_under_this_duration: u16,
}

#[before(ctos::TimeConfirm, priority = 2)]
#[register_to(YGOPRO_HANDLERS as ygopro_handler)]
fn on_time_confirm(duel: &mut Duel, attachment: &mut TimeLimit, player: PlayerIndex, config: Configuration) {
    if duel.host_info.time_limit == 0 { return; }
    if Some(player) != duel.last_response { return; }
    let time_elapsed = attachment.time_elapsed as u16;
    let Some(duel_player) = duel.get_mut(player) else { return; };
    if time_elapsed < config.ignore_small_time_under_this_duration && time_elapsed <= duel_player.time_compensator {
        duel_player.time_compensator -= time_elapsed;
        attachment.time_elapsed = 0;
    }
}

#[after(ctos::Response)]
#[register_to(YGOPRO_HANDLERS as ygopro_handler)]
fn on_response(duel: &mut Duel, player: PlayerIndex, response: &ctos::Response, config: Configuration) {
    if let Some(last_select_message) = &duel.last_select_message && response_is_meaningful(&response.response, last_select_message) {
        let add_deposit = config.add_small_time_deposit_after_operation;
        let time_limit = duel.host_info.time_limit;
        if let Some(duel_player) = duel.get_mut(player) {
            if duel_player.time_backed > 0 && duel_player.time_limit < time_limit {
                duel_player.time_compensator = duel_player.time_compensator.saturating_add(add_deposit);
            }
        }
    }
}

// #[after(gm::NewTurn)]
// #[register_to(YGOCORE_HANDLERS as ygocore_handler)]
// fn on_new_turn(duel: &mut SingleDuel) {
//     for duel_player in duel.players.iter_mut().flatten() {
//         duel_player.time_compensator = 0;
//     }
// }
