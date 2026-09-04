//! Extra scripts preloaded into every duel after creation.
//! 
//! It is often for loading server-limited special rules.    
//! File missing will not cause error.    
//! This plugin is defaultly enabled.
//!
//! # Examples
//!
//! Enable the module with a custom list of scripts:
//!
//! ```
//! use ygopro::plugin::preload_script::Configuration;
//!
//! let mut configuration = ygopro::Configuration::default();
//! configuration.enable_plugin_with_configuration(
//!     "ygopro::plugin::preload_script",
//!     Configuration { preloaded_scripts: vec!["./script/my_fantastic_rule.lua".to_string()] },
//! );
//! ```

use linkme::distributed_slice;

use ygopro_data::message::ctos;

use ygopro_derive::Configuration;
use ygopro_derive::before;
use ygopro_derive::register_to;

use crate::duel::Duel;
use crate::ygopro_handlers::Handler;

/// Name for activitating this module in the plugin system.
#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

/// Extra scripts to preload into every duel.
#[derive(Clone, Configuration)]
pub struct Configuration {
    /// Script paths preloaded into every duel after creation.
    #[config(not_from_env, default = "vec![\"./script/special.lua\".to_string()]")]
    pub preloaded_scripts: Vec<String>,
}

#[before(ctos::TpResult)]
#[register_to(crate::ygopro_handlers::YGOPRO_HANDLERS)]
fn on_tp_result(duel: &mut Duel, config: Configuration) {
    for script in &config.preloaded_scripts {
        if duel.preload_script(script) == 0 {
            log::debug!("Failed to preload script: {script}");
        }
    }
}
