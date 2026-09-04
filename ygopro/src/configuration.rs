//! Configuration for ygopro server.
//! 
//! [`Configuration`] is an `anymap::Map` wrapper that can be used to store plugin configurations.
//! Pass that to [`DuelHost::new`] to configure the server.
//! It can be seen as `HostInfo` extended in this project.
//! Any `Command`, `Handler` or `HandlerEx` will be ignored if its module name is not in the `enable_plugins` set.
//!
//! Example:
//! ```rust,no_run
//! let mut configuration = ygopro::Configuration::default();
//! configuration.enable_plugin("ygopro::plugin::soumatou");
//! let duel_host = ygopro::host::DuelHost::new(Default::default(), configuration);
//! // or start it directly with the cli toolkits:
//! # async fn run() {
//! ygopro::cli::start_local_server(0, duel_host).await;
//! # }
//! ```

use crate::plugin::*;

use ygopro_core_wrapper as core;

pub struct Configuration {
    pub no_mask: bool,
    pub seed_generator: Option<Box<dyn FnMut(u8) -> core::DuelSeed + Send>>,
    pub(crate) enable_plugins: hashbrown::HashSet<String>,
    pub(crate) configurations: anymap3::Map<dyn anymap3::CloneAny + Send>
}

impl Default for Configuration {
    fn default() -> Self {
        let mut configuration = Self {
            no_mask: false,
            seed_generator: None,
            enable_plugins: hashbrown::HashSet::new(),
            configurations: anymap3::Map::new()
        };
        for name in DEFAULT_ENABLED_PLUGINS {
            configuration.enable_plugin(name);
        }
        configuration
    }
}

impl Configuration {
    /// Generate a seed for the duel.
    /// That should be in a plugin for ygopro itself, but kept here as closure cannot be cloned.
    pub fn seed(&mut self, match_count: u8) -> core::DuelSeed {
        match &mut self.seed_generator {
            Some(generator) => generator(match_count),
            None => core::DuelSeed::None,
        }
    }

    /// Enable a plugin by its module path.
    /// If the plugin has a configuration, it will be initialized with its default value.
    pub fn enable_plugin(&mut self, plugin_name: &str) {
        for (name, init_config) in CONFIGURATIONS {
            if plugin_name == *name {
                init_config(&mut self.configurations).ok();
            }
        }
        self.enable_plugins.insert(plugin_name.to_string());
    }

    /// Enable a plugin by its module path and provide a configuration for it.
    pub fn enable_plugin_with_configuration<PluginConfiguration>(&mut self, plugin_name: &str, configuration: PluginConfiguration) where PluginConfiguration: Clone + Send + 'static {
        self.enable_plugins.insert(plugin_name.to_string());
        self.configurations.insert(configuration);
    }

    /// Disable a plugin by its module path.
    /// The configuration of the plugin will be remained from the configuration map.
    pub fn disable_plugin(&mut self, plugin_name: &str) {
        self.enable_plugins.remove(plugin_name);
    }
}
