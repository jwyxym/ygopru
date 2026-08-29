//! Room termination policy.
//! Whether a room should die when players leave is a deployment decision, not
//! duel core logic, so it lives in a plugin instead of `single_duel`.

use log::warn;
use linkme::distributed_slice;

use ygopro_data::constants::CorePlayer;
use ygopro_data::constants::Netplayer;
use ygopro_data::message::ctos;

use ygopro_derive::Configuration;
use ygopro_derive::after;
use ygopro_derive::command;
use ygopro_derive::register_to;

use crate::message as ygopro;
use crate::duel::SendTarget;
use crate::duel::Duel;
use crate::ygopro_handlers::Handler;
use crate::ygopro_handlers::HandlerEx;
use crate::ygopro_handlers::YGOPRO_HANDLERS;
use crate::ygopro_handlers::YGOPRO_HANDLERS_EX;

#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

#[derive(Clone, Configuration)]
pub struct Configuration {
    #[config(not_from_env)]
    pub terminate_when: SendTarget,
    #[config(default = "true")]
    pub terminate_when_match_end: bool
}

pub fn is_player_left(duel: &Duel, target: SendTarget) -> bool {
    match target {
        SendTarget::Single(netplayer) => match netplayer {
            Netplayer::Player(index) => duel.players.get(index as usize).map_or(true, Option::is_none),
            Netplayer::Observer(index) => !duel.observers.contains(index as usize),
            Netplayer::Undecided(index) => !duel.uninit_players.contains(index as usize),
            Netplayer::Unknown => { warn!("set terminate condition to unknown player"); is_player_left(duel, SendTarget::All) },
        },
        SendTarget::Except(_) => { warn!("set terminate condition to not supported except"); is_player_left(duel, SendTarget::All) },
        SendTarget::Core(core_player) => { log::warn!("set terminate condition to core player {core_player:?}, which is not supported for now"); false },
        SendTarget::All => is_player_left(duel, SendTarget::AllPlayer) && is_player_left(duel, SendTarget::AllObserver),
        SendTarget::AllPlayer => duel.players.iter().all(|player| player.is_none()),
        SendTarget::AllObserver => duel.observers.is_empty(),
        SendTarget::None => { warn!("a room is set to terminate in no case. this mean this room will be eternal."); false },
    }
}

#[after(ctos::LeaveGame)]
#[register_to(YGOPRO_HANDLERS)]
fn on_leave_game(duel: &mut Duel, config: Configuration) -> bool {
    is_player_left(duel, config.terminate_when)
}

#[after(ygopro::MatchEnd)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_match_end(configuration: Configuration) -> &'static str {
    if configuration.terminate_when_match_end { "terminate" } else { "continue" }
}

#[command]
#[register_to(crate::command::COMMANDS as crate::command::CommandHandler with &'static str)]
fn terminate() -> &'static str {
    "terminate"
}
