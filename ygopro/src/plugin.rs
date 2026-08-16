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

#[distributed_slice]
pub static DEFAULT_ENABLED_PLUGINS: [&'static str];

pub type InitConfiguration = fn(&mut anymap3::Map<dyn anymap3::CloneAny + Send>) -> Result<(), Box<dyn std::error::Error>>;
#[distributed_slice]
pub static CONFIGURATIONS: [(&'static str, InitConfiguration)];
