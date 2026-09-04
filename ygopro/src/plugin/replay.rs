//! Control how replays are distributed or stored.
//!
//! # Examples
//!
//! Enable the module with a replay mode:
//!
//! ```
//! use ygopro::plugin::replay::Configuration;
//!
//! let mut configuration = ygopro::Configuration::default();
//! configuration.enable_plugin_with_configuration(
//!     "ygopro::plugin::replay",
//!     Configuration { mode: ygopro_data::data::ReplayMode::WatcherNoSend },
//! );
//! ```

use linkme::distributed_slice;
use log::warn;

use ygopro_data::data::ReplayMode;
use ygopro_derive::Configuration;
use ygopro_derive::after;
use ygopro_derive::register_to;

use crate::message as ygopro;
use crate::duel::SendTarget;
use crate::ygopro_handlers::HandlerEx;
use crate::ygopro_handlers::YGOPRO_HANDLERS_EX;

/// Name for activitating this module in the plugin system.
#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

#[derive(Clone, Configuration)]
pub struct Configuration {
    #[config(not_from_env, default = "ReplayMode::empty()")]
    pub mode: ReplayMode
}

#[after(ygopro::GenerateReplay)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_generate_replay(configuration: Configuration, target: &mut SendTarget) {
    if configuration.mode.contains(ReplayMode::WatcherNoSend) {
        *target = SendTarget::AllPlayer
    }

    if configuration.mode.contains(ReplayMode::SaveInServer) {
        warn!("you set replay mode save in server, which is not supported for now.")
    }

    if configuration.mode.contains(ReplayMode::IncludeChat) {
        warn!("you set replay mode include chat, which is not supported for now.")
    }
}
