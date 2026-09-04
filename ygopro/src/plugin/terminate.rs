//! Room termination policy.
//! 
//! ygopro exits when all players leave, but in server scenarios, we want to terminate
//! the instance when duel ends. That plugin help set the termination policy for a room.
//!
//! # Examples
//!
//! Enable the module and terminate when all players leave:
//!
//! ```
//! use ygopro::plugin::terminate::Configuration;
//!
//! let mut configuration = ygopro::Configuration::default();
//! configuration.enable_plugin_with_configuration(
//!     "ygopro::plugin::terminate",
//!     Configuration { terminate_when: ygopro::duel::SendTarget::AllPlayer, terminate_when_match_end: true },
//! );
//! ```

use log::warn;
use linkme::distributed_slice;

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

/// Name for activitating this module in the plugin system.
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
