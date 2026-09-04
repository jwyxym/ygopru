//! Compensate for time lost to quick actions.
//!
//! At the start of each turn a player holds a secondary time wallet. Each
//! operation deposits some time into this wallet. When the player confirms
//! within a short duration and the wallet has enough balance, the elapsed time
//! is charged to the wallet instead of the player's `time_limit`. This
//! encourages quick operations and absorbs network jitter.
//! 
//! This plugin is defaultly enabled.
//!
//! # Warning
//!
//! In the original ygopro the wallet is initialized to the player's
//! `time_limit`, giving users effectively nearly three times the operation
//! time. The author considers this unreasonable, so the wallet starts at 0 by
//! default. Set `initial_time_balance` to `-1` to reproduce the original
//! behavior.
//!
//! # Examples
//!
//! Enable the module with a custom compensation policy:
//!
//! ```
//! use ygopro::plugin::time_compensator::Configuration;
//!
//! let mut configuration = ygopro::Configuration::default();
//! configuration.enable_plugin_with_configuration(
//!     "ygopro::plugin::time_compensator",
//!     Configuration { add_small_time_deposit_after_operation: 1, ignore_small_time_under_this_duration: 10, initial_time_balance: 0 },
//! );
//! ```

use linkme::distributed_slice;

use ygopro_data::message::ctos;
use ygopro_data::message::gm;
use ygopro_derive::Configuration;
use ygopro_derive::after;
use ygopro_derive::before;
use ygopro_derive::register_to;

use crate::duel::Duel;
use crate::duel::PlayerIndex;
use crate::duel::response_is_meaningful;
use crate::plugin::time_limit::TimeLimit;
use crate::ygocore_handlers::Handler as ygocore_handler;
use crate::ygocore_handlers::YGOCORE_HANDLERS;
use crate::ygopro_handlers::Handler as ygopro_handler;
use crate::ygopro_handlers::YGOPRO_HANDLERS;

/// Name for activitating this module in the plugin system.
#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

/// Policy for compensating away the time spent on quick operations.
#[derive(Clone, Configuration)]
pub struct Configuration {
    /// Seconds added to the player's compensation deposit for each meaningful operation.
    #[config(default = "1")]
    pub add_small_time_deposit_after_operation: u16,
    /// Elapsed time below this threshold (in seconds) may be ignored, funded by the compensation deposit.
    #[config(default = "10")]
    pub ignore_small_time_under_this_duration: u16,
    /// Seconds initially in the wallet at the start of each turn.
    /// `-1` means the wallet starts with the player's `time_limit`, matching the original ygopro behavior.
    #[config(default = "0")]
    pub initial_time_balance: i16,
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

#[after(gm::NewTurn)]
#[register_to(YGOCORE_HANDLERS as ygocore_handler)]
fn on_new_turn(duel: &mut Duel, config: Configuration) {
    let initial_balance = if config.initial_time_balance < 0 {
        duel.host_info.time_limit
    } else {
        config.initial_time_balance as u16
    };
    for duel_player in duel.players.iter_mut().flatten() {
        duel_player.time_compensator = initial_balance;
    }
}
