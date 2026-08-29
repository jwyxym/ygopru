//! Extra scripts preloaded into every core duel after creation.
//! Which scripts a server ships is a deployment decision, not duel core logic,
//! so it lives in a plugin instead of `single_duel`.

use linkme::distributed_slice;

use ygopro_data::message::ctos;

use ygopro_derive::Configuration;
use ygopro_derive::before;
use ygopro_derive::register_to;

use crate::duel::Duel;
use crate::ygopro_handlers::Handler;

#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

#[derive(Clone, Configuration)]
pub struct Configuration {
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
