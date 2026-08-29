use linkme::distributed_slice;

use ygopro_handler::sync_handler::SyncHandler;

use crate::duel::Duel;
use crate::ygopro_handlers::State;


pub type CommandHandler = SyncHandler<ygopro_handler::extract::Request<[u8; 8], ()>, State<Duel>, ygopro_handler::extract::Response<()>>;

#[distributed_slice]
pub static COMMANDS: [fn() -> (&'static str, CommandHandler)];
