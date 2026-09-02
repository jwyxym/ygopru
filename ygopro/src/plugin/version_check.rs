//! Stop user with wrong version client from joining duel.

use std::sync::LazyLock;

use ygopro_data::constants::ErrorMessage;
use ygopro_data::message::ctos;
use ygopro_data::message::stoc;
use ygopro_handler::StopFlag;

use crate::ygopro_handlers::Handler;
use crate::ygopro_handlers::YGOPRO_HANDLERS;

pub static PRO_VERSION: LazyLock<u16> = LazyLock::new(|| {
    let major = env!("CARGO_PKG_VERSION_MAJOR").parse::<u16>().unwrap();
    let minor = env!("CARGO_PKG_VERSION_MINOR").parse::<u16>().unwrap();
    let patch = env!("CARGO_PKG_VERSION_PATCH").parse::<u16>().unwrap();
    (major << 12) + (minor << 4) + patch
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
