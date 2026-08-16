//! Time added back to a player after each operation.
//! How much time a player is compensated for is a deployment decision, not
//! duel core logic, so it lives in a plugin instead of `single_duel`.

use linkme::distributed_slice;

use ygopro_data::message::ctos;
use ygopro_data::message::gm;
use ygopro_derive::Configuration;
use ygopro_derive::after;
use ygopro_derive::register_to;

use crate::common::response_is_meaningful;
use crate::single_duel::PlayerIndex;
use crate::single_duel::SingleDuel;
use crate::single_duel::ygopro_handlers::Handler;

#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

#[derive(Clone, Configuration)]
pub struct Configuration {
    #[config(default = "1")]
    pub add_time_after_operation: u16,
    pub max_add_time_each_turn: u16,
}

#[after(ctos::Response)]
#[register_to(crate::single_duel::ygopro_handlers::YGOPRO_HANDLERS)]
fn on_response(duel: &mut SingleDuel, player: PlayerIndex, response: &ctos::Response, config: Configuration) {
    if let Some(last_select_message) = &duel.last_select_message && response_is_meaningful(&response.response, last_select_message) {
        let add_time = config.add_time_after_operation;
        let time_limit = duel.host_info.time_limit;
        if let Some(duel_player) = duel.get_player_mut_index(player) {
            if duel_player.time_backed > 0 && duel_player.time_limit < time_limit {
                duel_player.time_limit = duel_player.time_limit.saturating_add(add_time);
                duel_player.time_backed = duel_player.time_backed.saturating_sub(add_time);
            }
        }
    }
}

fn reset_time_backed(duel: &mut SingleDuel, config: &Configuration) {
    let time_limit = duel.host_info.time_limit;
    if time_limit > 0 {
        let time_backed = if config.max_add_time_each_turn == 0 { if config.add_time_after_operation > 0 { time_limit } else { 0 } } else { config.max_add_time_each_turn };
        for duel_player in duel.players.iter_mut().flatten() {
            duel_player.time_backed = time_backed;
        }
    }
}

#[after(ctos::TpResult)]
#[register_to(crate::single_duel::ygopro_handlers::YGOPRO_HANDLERS)]
fn on_tp_result(duel: &mut SingleDuel, config: Configuration) {
    reset_time_backed(duel, &config);
}

#[after(gm::NewTurn)]
#[register_to(crate::single_duel::ygocore_handlers::YGOCORE_HANDLERS as crate::single_duel::ygocore_handlers::Handler)]
fn on_new_turn(duel: &mut SingleDuel, config: Configuration) {
    reset_time_backed(duel, &config);
}
