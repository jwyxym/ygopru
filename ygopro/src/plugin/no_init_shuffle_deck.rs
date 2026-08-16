use ygopro_derive::before;
use ygopro_derive::register_to;
use ygopro_handler::StopFlag;

use crate::message as ygopro;
use crate::single_duel::ygopro_handlers::HandlerEx;
use crate::single_duel::ygopro_handlers::YGOPRO_HANDLERS_EX;

pub static NAME: &'static str = module_path!();

#[before(ygopro::FirstShuffle)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_first_shuffle(stop: &mut StopFlag) {
    stop.0 = true
}

