//! Disable the initial shuffle of the deck in ygopro.
//! 
//! In ygopro, when duel start, we have an init shuffle of the deck. That plugin will disable it.
//! That's often useful for replaying a duel with a known seed, or for other testing purposes.
//!
//! # Examples
//!
//! Enable the module:
//!
//! ```
//! let mut configuration = ygopro::Configuration::default();
//! configuration.enable_plugin("ygopro::plugin::no_init_shuffle_deck");
//! ```

use ygopro_derive::before;
use ygopro_derive::register_to;
use ygopro_handler::StopFlag;

use crate::message as ygopro;
use crate::ygopro_handlers::HandlerEx;
use crate::ygopro_handlers::YGOPRO_HANDLERS_EX;

/// Name for activitating this module in the plugin system.
pub static NAME: &'static str = module_path!();

#[before(ygopro::FirstShuffle)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_first_shuffle(stop: &mut StopFlag) {
    stop.0 = true
}

