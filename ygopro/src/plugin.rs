//! Plugin system for ygopro.
//!
//! A plugin is a loose concept, there is no trait named `Plugin`. A plugin module usually
//! contains a `NAME` which is the module path used to enable the plugin, together with a set
//! of handlers, configurations, and attachments. Some plugins are enabled by default.
//! In principle, a plugin produces no effect at all when it is not enabled.
//! See [`Configuration`](crate::Configuration) for how to enable a plugin.
//!
//! # Examples
//!
//! Register a plugin named `my_fantastic_plugin` in its own module:
//!
//! ```rust
//! // my_fantastic_plugin.rs
//! use linkme::distributed_slice;
//! use ygopro_derive::Configuration;
//!
//! pub static NAME: &'static str = module_path!();
//! 
//! #[derive(Clone, Configuration)]
//! pub struct Configuration {
//!     #[config(not_from_env)]
//!     pub mode: u8
//! }
//! ```

use linkme::distributed_slice;

pub mod bo;
pub mod no_init_shuffle_deck;
pub mod preload_script;
pub mod replay;
pub mod reconnect;
pub mod soumatou;
pub mod terminate;
pub mod time_backed;
pub mod time_compensator;
pub mod time_limit;
pub mod version_check;

/// A list contains plugin names that are enabled by default. 
/// Declare NAME with linkme to enable a plugin by default.
/// 
/// ```rust
/// use linkme::distributed_slice;
///
/// #[distributed_slice(ygopro::plugin::DEFAULT_ENABLED_PLUGINS)]
/// pub static NAME: &'static str = module_path!();
/// ``` 
#[distributed_slice]
pub static DEFAULT_ENABLED_PLUGINS: [&'static str];

/// Function that initializes a plugin's configuration and inserts it into the configuration map.
/// This is generated and registered by the [`Configuration`] derive.
pub type InitConfiguration = fn(&mut anymap3::Map<dyn anymap3::CloneAny + Send>) -> Result<(), Box<dyn std::error::Error>>;

/// A list maps a plugin name to its [`InitConfiguration`].
/// Populated automatically by the [`Configuration`] derive, which registers one entry for each configurable plugin.
#[distributed_slice]
pub static CONFIGURATIONS: [(&'static str, InitConfiguration)];
