use ygopro_derive::Configuration;
use ygopro_derive::after;
use ygopro_derive::register_to;
use ygopro_handler::IntoResponse;

use crate::common::Response;
use crate::message as ygopro;
use crate::single_duel::SingleDuel;
use crate::single_duel::ygopro_handlers::HandlerEx;
use crate::single_duel::ygopro_handlers::YGOPRO_HANDLERS_EX;

pub static NAME: &'static str = module_path!();

#[derive(Clone, Configuration)]
pub struct Configuration {
    pub override_best_of: u8
}

#[after(ygopro::JudgeContinueMatch)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_judge_continue_match(duel: &mut SingleDuel, configuration: Configuration, response: &mut Response) {
    if configuration.override_best_of == 0 { return }
    let end_count = configuration.override_best_of as usize;
    let end_win_count = (configuration.override_best_of + 1) / 2;
    let mut player_wins = [0, 0];
    for winner in &duel.duel_winner {
        if let Some(winner) = winner { player_wins[*winner as usize] += 1 }
    }
    let should_match_end = duel.duel_winner.len() >= end_count || player_wins[0] >= end_win_count || player_wins[1] >= end_win_count;
    *response = (if should_match_end { "terminate" } else { "continue" }).into_response()
}
