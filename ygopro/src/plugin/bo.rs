//! Replace the default best-of-3 rule with a custom best-of-N rule.
//!
//! # Examples
//!
//! Enable the module with a best-of-5 rule:
//!
//! ```
//! use ygopro::plugin::bo::Configuration;
//!
//! let mut configuration = ygopro::Configuration::default();
//! configuration.enable_plugin_with_configuration(
//!     "ygopro::plugin::bo",
//!     Configuration { override_best_of: 5 },
//! );
//! ```


use ygopro_derive::Configuration;
use ygopro_derive::after;
use ygopro_derive::register_to;
use ygopro_handler::IntoResponse;

use crate::message as ygopro;
use crate::single_duel::SingleDuel;
use crate::single_duel::ygopro_handlers::HandlerEx;
use crate::single_duel::ygopro_handlers::SINGLE_DUEL_YGOPRO_HANDLERS_EX;
use crate::ygopro_handlers::Response;

/// Name for activitating this module in the plugin system.
pub static NAME: &'static str = module_path!();

/// Policy for a best-of-N match.
#[derive(Clone, Configuration)]
pub struct Configuration {
    /// Number of duels in a match. A match ends once a player wins more than half of them.
    /// `0` keeps the default Single/Match behavior.
    pub override_best_of: u8
}

#[after(ygopro::JudgeContinueMatch)]
#[register_to(SINGLE_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_judge_continue_match(duel: &mut SingleDuel, configuration: Configuration, response: &mut Response) {
    if configuration.override_best_of == 0 { return }
    let end_count = configuration.override_best_of as usize;
    let end_win_count = (configuration.override_best_of + 1) / 2;
    let mut player_wins = [0, 0];
    for winner in &duel.duel_winner {
        if let Some(winner) = winner { player_wins[winner.0 as usize] += 1 }
    }
    let should_match_end = duel.duel_winner.len() >= end_count || player_wins[0] >= end_win_count || player_wins[1] >= end_win_count;
    *response = (if should_match_end { "terminate" } else { "continue" }).into_response()
}
