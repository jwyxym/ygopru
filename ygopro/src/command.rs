//! Send command to duel to control duel behavior.
//! 
//! Command can be seen as a ctos ex message. It just send a message to duel and trigger that handler.
//! 
//! # Examples
//! 
//! Declare a command in a plugin module:
//! 
//! ```rust,no_run
//! use std::any::Any;
//! use ygopro_derive::command;
//! use ygopro_derive::register_to;
//!
//! #[command]
//! #[register_to(ygopro::command::COMMANDS as ygopro::command::CommandHandler with &'static str)]
//! fn my_command(duel: &mut ygopro::duel::Duel, arguments: &mut Box<dyn Any + Send>) {
//!     // ...
//! }
//!
//! // Then you can queue the command on a duel:
//! # fn call(duel: &ygopro::duel::Duel) {
//! duel.queue_command("my_command", Some(Box::new(0u8)));
//! # }
//! ```
//! 
//! The command's argument payload is a boxed arbitrary value, sent together with the command.
//! It is accessed with a `&mut` reference, so only `Send` is required of the payload.
//! 
//! 

use std::any::Any;

use linkme::distributed_slice;

use ygopro_handler::sync_handler::SyncHandler;

use crate::duel::Duel;
use crate::ygopro_handlers::State;


/// Handler of a command, carrying a boxed argument payload and accessing the [`Duel`].
pub type CommandHandler = SyncHandler<Box<dyn Any + Send>, State<Duel>, ygopro_handler::extract::Response<()>>;

/// A list maps each command name to its [`CommandHandler`] builder.
/// It is populated by the [`command`] attribute on `#[register_to(COMMANDS)]`.
#[distributed_slice]
pub static COMMANDS: [fn() -> (&'static str, CommandHandler)];
