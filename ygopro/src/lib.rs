//! YGOPro, rewritten in rust version.
//! 
//! Here should be some very big readme.

pub mod single_duel;
pub mod tag_duel;
pub mod duel;
pub mod host;
pub mod managers;
pub mod plugin;
pub mod message;
pub mod command;
pub mod cli;
pub mod ygopro_handlers;
pub mod ygocore_handlers;
pub mod player;
mod configuration;
#[cfg(feature = "zip")]
pub mod ypk;

pub use configuration::Configuration as Configuration;
pub use plugin::version_check::PRO_VERSION as PRO_VERSION;
pub use host::DuelHost as DuelHost;

use std::ffi::CStr;
use std::os::raw::c_char;
use managers::*;

#[macro_use] extern crate ygopro_derive;

pub fn init() {
    managers::config_manager::init();
    managers::i18n::init();
    #[cfg(feature = "zip")]
    ypk::archive_manager::init();
    managers::data_manager::init();
    managers::deck_manager::init();
    init_core();
}

pub fn init_core() {
    unsafe {
        ygopro_core_wrapper::set_script_reader(Some(data_manager::script_reader));
        ygopro_core_wrapper::set_card_reader(Some(data_manager::card_reader));
        ygopro_core_wrapper::set_message_handler(Some(core_message_handler));
    }
}

extern "C" fn core_message_handler(pduel: isize, message_type: u32) -> u32 {
    let mut buffer = [0u8; 1024];
    unsafe { ygopro_core_wrapper::get_log_message(pduel, buffer.as_mut_ptr()); }
    let c_message = unsafe { CStr::from_ptr(buffer.as_ptr() as *const c_char) };
    log::debug!("core message[{}]: {}", message_type, c_message.to_string_lossy());
    0
}
