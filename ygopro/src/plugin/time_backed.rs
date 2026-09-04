//! Give players extra time while performing operations.
//!
//! Each turn a player is granted a budget of time that may be added back to
//! their clock. Every time the player performs a meaningful operation, some
//! time is added to their remaining time limit, drawing from that budget. This
//! reduces the risk of a player timing out during a long series of operations,
//! without letting a single turn extend forever.
//! 
//! This plugin is defaultly enabled.
//!
//! # Examples
//!
//! Enable the module with a custom time-back policy:
//!
//! ```
//! use ygopro::plugin::time_backed::Configuration;
//!
//! let mut configuration = ygopro::Configuration::default();
//! configuration.enable_plugin_with_configuration(
//!     "ygopro::plugin::time_backed",
//!     Configuration { add_time_after_operation: 1, max_add_time_each_turn: 600 },
//! );
//! ```

use linkme::distributed_slice;

use ygopro_data::message::ctos;
use ygopro_data::message::gm;
use ygopro_derive::Configuration;
use ygopro_derive::after;
use ygopro_derive::register_to;

use crate::duel::Duel;
use crate::duel::response_is_meaningful;
use crate::duel::PlayerIndex;
use crate::ygopro_handlers::Handler;

/// Name for activitating this module in the plugin system.
#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

/// Policy for granting extra time back to a player.
#[derive(Clone, Configuration)]
pub struct Configuration {
    /// Seconds added to the player's clock for each meaningful operation.
    #[config(default = "1")]
    pub add_time_after_operation: u16,
    /// Maximum seconds a player may recover in a single turn before the budget is exhausted.
    /// `0` means the same as the player's `time_limit`, so the whole time limit may be recovered.
    pub max_add_time_each_turn: u16,
}

#[after(ctos::Response)]
#[register_to(crate::ygopro_handlers::YGOPRO_HANDLERS)]
fn on_response(duel: &mut Duel, player: PlayerIndex, response: &ctos::Response, config: Configuration) {
    if let Some(last_select_message) = &duel.last_select_message && response_is_meaningful(&response.response, last_select_message) {
        let add_time = config.add_time_after_operation;
        let time_limit = duel.host_info.time_limit;
        if let Some(duel_player) = duel.get_mut(player) {
            if duel_player.time_backed > 0 && duel_player.time_limit < time_limit {
                duel_player.time_limit = duel_player.time_limit.saturating_add(add_time);
                duel_player.time_backed = duel_player.time_backed.saturating_sub(add_time);
            }
        }
    }
}

fn reset_time_backed(duel: &mut Duel, config: &Configuration) {
    let time_limit = duel.host_info.time_limit;
    if time_limit > 0 {
        let time_backed = if config.max_add_time_each_turn == 0 { if config.add_time_after_operation > 0 { time_limit } else { 0 } } else { config.max_add_time_each_turn };
        for duel_player in duel.players.iter_mut().flatten() {
            duel_player.time_backed = time_backed;
        }
    }
}

#[after(ctos::TpResult)]
#[register_to(crate::ygopro_handlers::YGOPRO_HANDLERS)]
fn on_tp_result(duel: &mut Duel, config: Configuration) {
    reset_time_backed(duel, &config);
}

#[after(gm::NewTurn)]
#[register_to(crate::ygocore_handlers::YGOCORE_HANDLERS as crate::ygocore_handlers::Handler)]
fn on_new_turn(duel: &mut Duel, config: Configuration) {
    reset_time_backed(duel, &config);
}
