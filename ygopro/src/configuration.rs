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
    pub fn seed(&mut self, match_count: u8) -> core::DuelSeed {
        match &mut self.seed_generator {
            Some(generator) => generator(match_count),
            None => core::DuelSeed::None,
        }
    }

    pub fn enable_plugin(&mut self, plugin_name: &str) {
        for (name, init_config) in CONFIGURATIONS {
            if plugin_name == *name {
                init_config(&mut self.configurations).ok();
            }
        }
        self.enable_plugins.insert(plugin_name.to_string());
    }

    pub fn enable_plugin_with_configuration<PluginConfiguration>(&mut self, plugin_name: &str, configuration: PluginConfiguration) where PluginConfiguration: Clone + Send + 'static {
        self.enable_plugins.insert(plugin_name.to_string());
        self.configurations.insert(configuration);
    }

    pub fn disable_plugin(&mut self, plugin_name: &str) {
        self.enable_plugins.remove(plugin_name);
    }
}
