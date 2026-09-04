//! Stop user with wrong version client from joining duel.
//! 
//! This version check is strict: No bias is allowed. 
//! Understream service should disable this plugin if it wants to allow different version clients to join duel. 
//! This plugin is defaultly enabled.
//!
//! # Examples
//!
//! Enable the module:
//!
//! ```
//! let mut configuration = ygopro::Configuration::default();
//! configuration.enable_plugin("ygopro::plugin::version_check");
//! ```

use std::sync::LazyLock;

use linkme::distributed_slice;

use ygopro_data::constants::ErrorMessage;
use ygopro_data::message::ctos;
use ygopro_data::message::stoc;
use ygopro_handler::StopFlag;

use crate::ygopro_handlers::Handler;
use crate::ygopro_handlers::YGOPRO_HANDLERS;

/// Name for activitating this module in the plugin system.
#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

pub static PRO_VERSION: LazyLock<u16> = LazyLock::new(|| {
    let major = env!("CARGO_PKG_VERSION_MAJOR").parse::<u16>().unwrap();
    let minor = env!("CARGO_PKG_VERSION_MINOR").parse::<u16>().unwrap();
    let patch = env!("CARGO_PKG_VERSION_PATCH").parse::<u16>().unwrap();
    let hex_minor = minor % 10 + minor / 10 * 16;
    (major << 12) + (hex_minor << 4) + patch
});

#[before(ctos::JoinGame)]
#[register_to(YGOPRO_HANDLERS)]
pub fn before_join_game(join_game: &ctos::JoinGame, stop_flag: &mut StopFlag) -> Result<(), stoc::Message> {
    if join_game.version != *PRO_VERSION {
        stop_flag.0 = true;
        return Err(stoc::ErrorMessage { err: ErrorMessage::VersionError(*PRO_VERSION) }.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_parts_are_numbers() {
        let _: u16 = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap();
        let _: u16 = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap();
        let _: u16 = env!("CARGO_PKG_VERSION_PATCH").parse().unwrap();
    }
}
