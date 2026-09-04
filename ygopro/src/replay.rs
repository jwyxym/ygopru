//! Replay a saved `.yrp` match.
//!
//! A [`ReplayHost`] builds a live duel whose players are fake — their decks come
//! from the replay — and lets a client join mid-duel as an observer. The stored
//! responses are fed back one at a time, but only once the duel is actually
//! waiting for that response, so the observer watches the match unfold.
//!
//! Note: this drives the duel with a counterfeit [`Duel`] built by
//! [`counterfeit_duel`], skipping the whole ygopro start-flow and going straight
//! into the match. If you need the full handshake and game-entry flow, see the
//! implementation in `ygopro-toolkits`.
//!
//! # Examples
//!
//! ```no_run
//! use std::io::Cursor;
//!
//! use binrw::BinRead;
//! use ygopro::replay::ReplayHost;
//! use ygopro_data::data::Replay;
//!
//! # async fn run() {
//! let bytes = std::fs::read("match.yrp").unwrap();
//! let replay = Replay::read_le(&mut Cursor::new(bytes)).unwrap();
//! let mut host = ReplayHost::new(replay);
//! host.drain().await;
//! # }
//! ```

use std::any::Any;
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;

use futures::Stream;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use futures::StreamExt;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::UnboundedReceiverStream;
use ygopro_core_wrapper as core;
use ygopro_data::complex::Complex;
use ygopro_data::constants::DuelStage;
use ygopro_data::constants::Netplayer;
use ygopro_data::data::{Deck, Replay, Response};
use ygopro_data::message::{ctos, stoc};
use ygopro_data::message::gm;
use ygopro_data::message::gm::GameMessage;
use ygopro_data::string::FixedLengthString;
use ygopro_derive::command;
use ygopro_derive::register_to;
use ygopro_handler::RoomProvider;

use crate::{Configuration, DuelHost};
use crate::duel::*;
use crate::player::*;
use crate::single_duel::SingleDuel;
use crate::tag_duel::TagDuel;
use crate::tag_duel::TeamIndex;

/// Build a live duel from a replay, with players backed by the replay's decks.
///
/// The duel's state is set directly to the values it would hold after the
/// pre-game flow, so it goes straight into the match. The players are registered
/// with the duel's sender so their stoc output can be observed; the returned
/// receivers let the driver wait for the duel to request a response instead of
/// guessing a timing. Replay relies on the [`crate::plugin::soumatou`] plugin to
/// keep a mid-duel observer up to date.
pub fn counterfeit_duel(replay: Replay) -> (UnboundedSender<Request>, Vec<UnboundedReceiver<Complex<stoc::Message>>>, JoinHandle<()>) {
    let mut configuration: Configuration = Default::default();
    configuration.enable_plugin(module_path!());
    configuration.no_mask = true;
    let mut duel = Duel::new(replay.host_info(), configuration);
    duel.configuration.enable_plugin(crate::plugin::soumatou::NAME);
    duel.core = core::Duel::new(core::DuelSeed::Complicated(replay.header.seed_sequence));
    duel.max_player_count = if replay.header.is_tag() { 4 } else { 2 };
    let mut receivers = vec![];
    receivers.push(counterfeit_player(replay.body.host_name, replay.body.host_deck.into(), &mut duel));
    if replay.header.is_tag() {
        receivers.push(counterfeit_player(replay.body.tag_host_name.unwrap(), replay.body.tag_host_deck.unwrap().into(), &mut duel));
    }
    receivers.push(counterfeit_player(replay.body.client_name, replay.body.client_deck.into(), &mut duel));
    if replay.header.is_tag() {
        receivers.push(counterfeit_player(replay.body.tag_client_name.unwrap(), replay.body.tag_client_deck.unwrap().into(), &mut duel));
    }
    duel.stage = DuelStage::Dueling;
    duel.sender.send(stoc::DuelStart.into(), crate::duel::SendTarget::All);
    let sender = duel.request_sender.clone();
    let join_handle = if replay.header.is_tag() {
        TagDuel { duel, first_attack_team: Some(TeamIndex::Team1), duel_winner: vec![], current_turn_player: None, surrender: [false; 4] }.run()
    } else {
        SingleDuel { duel, first_attack_player: Some(PlayerIndex(0)), duel_winner: vec![] }.run()
    }.expect("duel already started");
    sender.send(crate::duel::Request::MessageEx(
        crate::ygopro_handlers::RequestEx {
            message: crate::message::DuelInit.into(),
            extra: crate::duel::SendTarget::All,
        }
    )).ok();
    (sender, receivers, join_handle)
}

fn counterfeit_player(name: FixedLengthString<20>, deck: Deck, duel: &mut Duel) -> UnboundedReceiver<Complex<stoc::Message>> {
    let (stoc_sender, stoc_receiver) = mpsc::unbounded_channel();
    let index = duel.players.len();
    duel.players.push(Some(DuelPlayer {
        player: BaseDuelPlayer {
            name,
            stoc_sender: stoc_sender.clone(),
            state: AllowMessage::Any
        },
        ready: false,
        deck,
        hand: None,
        deck_error: None,
        time_limit: 999,
        time_compensator: 0,
        time_backed: 0
    }));
    duel.sender.set_player(index, stoc_sender);
    stoc_receiver
}

/// Replay a saved duel to a connecting observer.
///
/// It runs a fake duel (see [`counterfeit_duel`]) and drives it with the stored
/// responses, waiting for the duel to actually request each one, so the observer
/// sees the match unfold rather than a pre-computed result.
pub struct ReplayHost {
    host: DuelHost,
    replay: Replay,
    pos: usize,
    stream: futures::stream::SelectAll<UnboundedReceiverStream<Complex<stoc::Message>>>
}

impl Deref for ReplayHost {
    type Target = DuelHost;

    fn deref(&self) -> &Self::Target {
        &self.host
    }
}

impl DerefMut for ReplayHost {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.host
    }
}

impl ReplayHost {
    /// Start driving the duel for `replay`. The duel itself runs on a background
    /// task; this returns once it has been spawned.
    pub fn new(replay: Replay) -> Self {
        let (ctos_sender, receivers, handle) = counterfeit_duel(replay.clone());
        let (finished_sender, _) = watch::channel(false);
        let finished_sender_for_host = finished_sender.clone();
        tokio::spawn(async move {
            let _ = handle.await;
            finished_sender.send(true).ok();
        });
        let stream = futures::stream::select_all(receivers.into_iter().map(UnboundedReceiverStream::new).collect::<Vec<_>>());
        let host = DuelHost { ctos_sender, finished_sender: finished_sender_for_host };
        Self { host, replay, pos: 0, stream }
    }

    /// Feed one stored response, waiting until the duel is actually waiting for
    /// it. Returns `true` once every stored response has been fed.
    pub async fn step(&mut self) -> bool {
        let Some(data) = self.replay.datas.get(self.pos) else { return true; };
        let response = data.data.clone();
        loop {
            match self.stream.next().await {
                Some(msg) => match msg.deref() {
                    stoc::Message::GameMessage(game_message) if game_message.message.waiting_for().is_some() => {
                        let waiting: Netplayer = game_message.message.waiting_for().unwrap().into();
                        self.ctos_sender.send(crate::duel::Request::Message(crate::ygopro_handlers::Request {
                            message: ctos::Response { response: response.clone() }.into(),
                            extra: waiting,
                        })).ok();
                        break;
                    },
                    stoc::Message::GameMessage(game_message)
                        if gm::MessageType::from(&game_message.message) == gm::MessageType::Retry => {
                        log::warn!("replay desynced");
                        return true;
                    },
                    _ => (),
                },
                None => return true,
            }
        }
        self.pos = self.pos + 1;
        self.is_finished() 
    }

    /// Feed all stored responses in order.
    pub async fn drain(&mut self) {
        while !self.step().await {}
    }

    /// Whether every stored response has been fed.
    pub fn is_finished(&self) -> bool {
        self.pos >= self.replay.datas.len()
    }

    /// Unwrap into the underlying [`DuelHost`].
    pub fn into_inner(self) -> DuelHost {
        self.host
    }
}

#[command]
#[register_to(crate::command::COMMANDS as crate::command::CommandHandler with &'static str)]
fn send_response(duel: &mut Duel, arguments: &mut Box<dyn Any + Send>){
    let Some(response) = arguments.downcast_ref::<Response>() else { return };
    let Some(waiting) = duel.last_select_message.as_ref().and_then(|select| select.waiting_for()) else { return };
    duel.queue_request(ctos::Response { response: response.clone() }, waiting.into());
}

impl RoomProvider<ctos::Message, Complex<stoc::Message>> for ReplayHost {
    type ServerToClientStream = UnboundedReceiverStream<Complex<stoc::Message>>;
    type FinishFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

    fn add(&mut self, client_to_server_stream: impl Stream<Item = ctos::Message> + Unpin + Send + 'static) -> Self::ServerToClientStream {
        self.host.add(client_to_server_stream)
    }

    fn get_finish_signal(&mut self) -> Self::FinishFuture {
        <DuelHost as RoomProvider<ctos::Message, Complex<stoc::Message>>>::get_finish_signal(&mut self.host)
    }
}

impl RoomProvider<Complex<ctos::Message>, Complex<stoc::Message>> for ReplayHost {
    type ServerToClientStream = UnboundedReceiverStream<Complex<stoc::Message>>;
    type FinishFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

    fn add(&mut self, client_to_server_stream: impl Stream<Item = Complex<ctos::Message>> + Unpin + Send + 'static) -> Self::ServerToClientStream {
        self.host.add(client_to_server_stream)
    }

    fn get_finish_signal(&mut self) -> Self::FinishFuture {
        <DuelHost as RoomProvider<Complex<ctos::Message>, Complex<stoc::Message>>>::get_finish_signal(&mut self.host)
    }
}


