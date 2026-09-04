//! Send command to duel to control duel behavior.
//! 
//! Command can be seen as a ctos ex message. It just send a message to duel and trigger that handler.
//! 
//! # Examples
//! 
//! Declare a command in a plugin module:
//! 
//! ```rust,no_run
//! use ygopro_derive::command;
//! use ygopro_derive::register_to;
//!
//! #[command]
//! #[register_to(ygopro::command::COMMANDS as ygopro::command::CommandHandler with &'static str)]
//! fn my_command(duel: &mut ygopro::duel::Duel, arguments: &[u8; 8]) {
//!     // ...
//! }
//!
//! // Then you can queue the command on a duel:
//! # fn call(duel: &ygopro::duel::Duel) {
//! duel.queue_command("my_command", [0, 1, 2, 3, 4, 5, 6, 7]);
//! # }
//! ```
//! 
//! The `[u8; 8]` is the command's argument payload, sent together with the command.
//! Its layout may change in the future.
//! 
//! 

use linkme::distributed_slice;

use ygopro_handler::sync_handler::SyncHandler;

use crate::duel::Duel;
use crate::ygopro_handlers::State;


/// Handler of a command, carrying the command's `[u8; 8]` arguments and accessing the [`Duel`].
pub type CommandHandler = SyncHandler<ygopro_handler::extract::Request<[u8; 8], ()>, State<Duel>, ygopro_handler::extract::Response<()>>;

/// A list maps each command name to its [`CommandHandler`] builder.
/// It is populated by the [`command`](ygopro_derive::command) attribute on `#[register_to(COMMANDS)]`.
#[distributed_slice]
pub static COMMANDS: [fn() -> (&'static str, CommandHandler)];
