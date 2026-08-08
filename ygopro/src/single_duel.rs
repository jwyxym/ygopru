use std::ops::Deref;
use std::ops::DerefMut;

use bytes::BytesMut;
use log::warn;
use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;

use ygopro_core_wrapper as core;
use ygopro_data::complex::Complex;
use ygopro_data::constants::*;
use ygopro_data::data::*;
use ygopro_data::message::HostInfo;
use ygopro_data::message::gm::MaskedClone;
use ygopro_data::string::FixedLengthString;
use ygopro_data::message::ctos;
use ygopro_data::message::stoc;
use ygopro_data::message::gm;
use ygopro_data::message::gm::GameMessage;
use ygopro_handler::RoomProvider;
use ygopro_handler::Bundle;
use ygopro_handler::FromRequest;
use ygopro_handler::MessageKey;

use crate::common;
use crate::common::Configuration;
use crate::common::Response;
use crate::common::SendTarget;
use crate::common::State;

pub fn init() {
    ygopro_handlers::reset_processor();
    ygocore_handlers::reset_processor(); 
}


pub enum Request {
    Join { stoc_sender: mpsc::UnboundedSender<Complex<stoc::Message>> },
    Message(common::Request),
    TimerTick,
    /// call ygocore to push the game state.
    /// produced by duel start and ctos response.
    Evolve,
    /// resend all history to target observer.
    /// produced by observer join on middle of duel.
    Soumatou(Netplayer),
    Stop
}

type BaseDuelPlayer = common::DuelPlayer<Complex<stoc::Message>>;
pub struct DuelPlayer {
    player: BaseDuelPlayer,
    ready: bool,
    deck: Deck,
    hand: Option<Hand>,
    deck_error: Option<DeckError>,
    time_limit: u16,
    time_compensator: u16,
    time_backed: u16,
}

impl From<BaseDuelPlayer> for DuelPlayer {
    fn from(value: BaseDuelPlayer) -> Self {
        Self {
            player: value, 
            ready: false,
            deck: Deck::new(),
            hand: None,
            deck_error: None,
            time_limit: 0,
            time_compensator: 0,
            time_backed: 0,
        }
    }
}

impl Deref for DuelPlayer {
    type Target = BaseDuelPlayer;
    fn deref(&self) -> &Self::Target { &self.player }
}

impl DerefMut for DuelPlayer {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.player }
}

impl AsRef<BaseDuelPlayer> for DuelPlayer {
    fn as_ref(&self) -> &BaseDuelPlayer { &self.player }
}

impl AsMut<BaseDuelPlayer> for DuelPlayer {
    fn as_mut(&mut self) -> &mut BaseDuelPlayer { &mut self.player }
}

impl<Response> FromRequest<common::Request, State<SingleDuel>, Response> for &mut DuelPlayer where Request: Send + Sync, Response: Send {
    fn from_request(bundle: &mut Bundle<common::Request, State<SingleDuel>, Response>) -> Option<Self> {
        let player = bundle.state.duel.get_player_mut(bundle.request.extra)?;
        Some(unsafe { &mut *(player as *mut DuelPlayer) })
    }
}

/// PlayerIndex is the super strict version of Netplayer.
/// It only accepts Netplyaer(0) and Netplayer(1).
/// It supposed to work just as Netplayer works.
#[derive(Copy, Clone, Eq, PartialEq, Debug, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum PlayerIndex {
    Player1 = 0,
    Player2 = 1
}

impl PlayerIndex {
    pub fn opponent(self) -> Self {
        match self {
            PlayerIndex::Player1 => PlayerIndex::Player2,
            PlayerIndex::Player2 => PlayerIndex::Player1,
        }
    }
}

impl TryFrom<Netplayer> for PlayerIndex {
    type Error = ();

    fn try_from(value: Netplayer) -> Result<Self, Self::Error> {
        match value {
            Netplayer::Player(0) => Ok(PlayerIndex::Player1),
            Netplayer::Player(1) => Ok(PlayerIndex::Player2),
            _ => Err(())
        }
    }
}

impl From<PlayerIndex> for SendTarget {
    fn from(value: PlayerIndex) -> Self {
        let player: Netplayer = value.into();
        player.into()
    }
}

impl TryFrom<usize> for PlayerIndex {    
    type Error = ();
    
    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Player1),
            1 => Ok(Self::Player2),
            _ => Err(())
        }
    }
}

impl Into<Netplayer> for PlayerIndex {
    fn into(self) -> Netplayer {
        Netplayer::Player(self as u8)
    }
}

impl<State, Res> FromRequest<common::Request, State, Res> for PlayerIndex where State: Send, Res: Send {
    fn from_request(bundle: &mut Bundle<common::Request, State, Res>) -> Option<Self> {
        Self::try_from(bundle.request.extra).ok()
    }
}

struct SingleDuel {
    duel: common::Duel,
    players: [Option<DuelPlayer>; 2], 
    observers: Vec<Option<BaseDuelPlayer>>,
    first_attack_player: Option<PlayerIndex>,
    first_attack_decider: Option<PlayerIndex>,
    last_response: Option<PlayerIndex>,
    match_kill_card_code: i32,
    duel_count: u8,
    duel_winner: Vec<Option<PlayerIndex>>,
    time_elapsed: u16,
    // these fields are only for request_field.
    // that message are actually inner core.
    // that make ygopro works like srvpro, which make us think that should be a Room attachment instead.
    phase: Phase,
    deck_reversed: bool,
    turn_player: CorePlayer,
    last_select_message: Option<gm::Message>,
    // extended by rust ygopro
    response_buffer: BytesMut,
    core_request_buffer: BytesMut,
    configuration: Configuration,
    timer_task: Option<tokio::task::JoinHandle<()>>,
    last_init_player: Option<BaseDuelPlayer>,
    // replay recorder
    start_time: u32,
    messages: Vec<Complex<stoc::Message>>,
    masked_messages: Vec<Complex<stoc::Message>>,
    client_responses: Vec<ctos::Response>,
    // extended by actor models
    request_sender: mpsc::UnboundedSender<Request>,
    request_receiver: Option<mpsc::UnboundedReceiver<Request>>,
}

impl SingleDuel {
    pub fn new(host_info: HostInfo, mut configuration: Configuration) -> Self {
        let seed = configuration.seed(0);
        let (request_sender, request_receiver) = mpsc::unbounded_channel();
        Self {
            duel: common::Duel {
                host_player: Netplayer::Unknown,
                host_info,
                stage: DuelStage::Begin,
                duel: core::Duel::new(seed),
                name: FixedLengthString::allocate(),
                pass: FixedLengthString::allocate(),
            },
            players: [None, None],
            observers: Vec::new(),
            first_attack_player: None,
            last_response: None,
            first_attack_decider: None,
            phase: Phase::Draw,
            deck_reversed: false,
            turn_player: CorePlayer::FirstAttackPlayer,
            last_select_message: None,
            match_kill_card_code: 0,
            duel_count: 0,
            duel_winner: Vec::new(),
            time_elapsed: 0,
            start_time: 0,
            response_buffer: BytesMut::zeroed(core::SIZE_RETURN_VALUE),
            core_request_buffer: BytesMut::zeroed(core::SIZE_QUERY_BUFFER),
            configuration,
            messages: Vec::new(),
            masked_messages: Vec::new(),
            client_responses: Vec::new(),
            request_sender,
            request_receiver: Some(request_receiver),
            last_init_player: None,
            timer_task: None,
        }
    }

    pub fn run(mut self) -> Option<tokio::task::JoinHandle<()>> {
        let receiver = self.request_receiver.take()?;
        let mut stream = UnboundedReceiverStream::new(receiver);

        let handle = tokio::spawn(async move {
            let ygopro_processor = ygopro_handlers::YGOPRO_PROCESSOR.get().expect("Processor not initialized").load_full();
            let ygocore_processor = ygocore_handlers::YGOCORE_PROCESSOR.get().expect("Processor not initialized").load_full();
            let mut duel = self;

            while let Some(request) = stream.next().await {
                match request {
                    Request::Join { stoc_sender } => {
                        if duel.stage > DuelStage::Begin {
                            if ! duel.configuration.allow_join_after_start {
                                let error = stoc::ErrorMessage { err: ErrorMessage::JoinError(JoinError::HostRefused) }.into();
                                stoc_sender.send(Complex::from_message(error)).ok();
                                continue;
                            }
                        }
                        if duel.last_init_player.is_some() { warn!("Two players are trying to init in the same duel.") }
                        duel.last_init_player = Some(common::DuelPlayer::new(stoc_sender));
                    },
                    Request::TimerTick => {
                        if let Some(last_response) = duel.last_response && duel.host_info.time_limit > 0 {
                            duel.time_elapsed = duel.time_elapsed.saturating_add(1);
                            let timed_out = duel.get_player_index(last_response)
                                    .map_or(false, |player| duel.time_elapsed >= player.time_limit);
                            if timed_out {
                                let loser = duel.to_core_player(last_response);
                                duel.win_and_end(loser, WinReason::Timeout);
                            }
                        }
                    },
                    Request::Message(request) => {
                        if let Some(Some(allowed)) = duel.get_player(request.extra).map(|p| p.allow_message(&request.message)) {
                            warn!("Message type mismatch for player: {:?}, get {:?}, expected {:?}", request.extra, ctos::MessageType::from(&request.message), allowed);
                            continue;
                        }
                        let state = common::State { duel };
                        let bundle = Bundle { request, state, response: Default::default() };
                        let key = bundle.request.message_key();
                        let Bundle {
                            request,
                            state: common::State { duel: returned_duel },
                            response
                        } = ygopro_processor.process_bundle(bundle, key).await;
                        duel = returned_duel;
                        let position = request.extra;
                        match response {
                            ygopro_handler::extract::Response::Replace(message) => duel.send(message, position.into()),
                            ygopro_handler::extract::Response::ReplaceMultiple(messages) => {
                                let position = position.into();
                                for message in messages {
                                    duel.send(message, position);
                                };
                            },
                            ygopro_handler::extract::Response::Continue => {},
                            ygopro_handler::extract::Response::Swallow => {},
                            ygopro_handler::extract::Response::Stop => { break; }
                            ygopro_handler::extract::Response::Kick => duel.send(stoc::LeaveGame { pos: position }.into(), position.into())
                        };
                    }
                    Request::Evolve => {
                        let messages = ygocore_handlers::evolve(&mut duel);
                        if let Some(core_player) = messages.last().and_then(|m| m.waiting_for()) {
                            ygocore_handlers::set_waiting(&mut duel, core_player);
                        } else if messages.last().is_some_and(|m| matches!(m, gm::Message::Retry(_))) {
                            duel.client_responses.pop();
                            if let Some(core_player) = duel.last_response.map(|player| duel.to_core_player(player)) {
                                ygocore_handlers::set_waiting(&mut duel, core_player);
                            }
                        }
                        for message in messages {
                            let key = message.message_key();
                            if gm::MessageType::from(&message) == gm::MessageType::Retry && duel.configuration.terminate_when_retry { break; }
                            let request = ygocore_handlers::Request { message, extra: Netplayer::Unknown };
                            let state = common::State { duel };
                            let bundle = Bundle { request, state, response: Default::default() };
                            let Bundle {
                                request,
                                state: common::State { duel: mut returned_duel },
                                response
                            } = ygocore_processor.process_bundle(bundle, key).await;
                            returned_duel.send_game_message(request.message, response.target);
                            let (player, locations, sequence, query) = response.refresh;
                            returned_duel.refresh(player, locations, sequence, query);
                            duel = returned_duel;
                            if duel.stage == DuelStage::End {
                                break;
                            }
                        }
                    },
                    Request::Soumatou(player) => {
                        let target = player.into();
                        for message in &duel.masked_messages {
                            duel._send(message.clone(), target);
                        }
                    },
                    Request::Stop => {
                        break
                    }
                }
            }
        });

        Some(handle)
    }

    pub fn get_player(&self, player: Netplayer) -> Option<&DuelPlayer> {
        match player {
            Netplayer::Player(netplayer) => {
                self.players[netplayer as usize].as_ref()
            }
            _ => None,
        }
    }

    pub fn get_player_mut(&mut self, player: Netplayer) -> Option<&mut DuelPlayer> {
        match player {
            Netplayer::Player(netplayer) => {
                self.players[netplayer as usize].as_mut()
            }
            _ => None,
        }
    }

    pub fn get_player_index(&self, player: PlayerIndex) -> Option<&DuelPlayer> {
        self.players[player as u8 as usize].as_ref()
    }

    pub fn get_player_mut_index(&mut self, player: PlayerIndex) -> Option<&mut DuelPlayer> {
        self.players[player as u8 as usize].as_mut()        
    }

    pub fn observer_count(&self) -> u16 {
        self.observers.iter().fold(0, |s, v| { s + if v.is_some(){ 1 } else { 0 } }) as u16
    }

    pub fn insert_observer(&mut self, player: BaseDuelPlayer) -> Netplayer {
        let slot = self.observers.iter().position(|v| v.is_none());
        let position = match slot {
            Some(slot) => {
                self.observers[slot] = Some(player);
                slot as u8
            }
            None => {
                self.observers.push(Some(player));
                self.observers.len() as u8 - 1
            }
        };
        Netplayer::Observer(position)
    }

    pub fn to_core_player(&self, player: PlayerIndex) -> CorePlayer {
        let first_attack_player = self.first_attack_player.unwrap_or(PlayerIndex::Player1);
        let core_player: CorePlayer = match player {
            PlayerIndex::Player1 => CorePlayer::FirstAttackPlayer,
            PlayerIndex::Player2 => CorePlayer::SecondAttackPlayer,
        };
        if first_attack_player == PlayerIndex::Player1 {
            core_player
        } else {
            core_player.opponent()
        }
    }

    pub fn to_player_index(&self, core_player: CorePlayer) -> Option<PlayerIndex> {
        let first_attack_player = self.first_attack_player.unwrap_or(PlayerIndex::Player1);
        let core_player = match first_attack_player {
            PlayerIndex::Player1 => core_player,
            _ => core_player.opponent()
        };
        let index = match core_player {
            CorePlayer::FirstAttackPlayer => PlayerIndex::Player1,
            CorePlayer::SecondAttackPlayer => PlayerIndex::Player2,
            _ => return None
        };
        Some(index)
    }

    pub fn to_net_player(&self, core_player: CorePlayer) -> Netplayer {
        let first_attack_player = self.first_attack_player.unwrap_or(PlayerIndex::Player1);
        let core_player = match first_attack_player {
            PlayerIndex::Player1 => core_player,
            _ => core_player.opponent()
        };
        match core_player {
            CorePlayer::FirstAttackPlayer => Netplayer::Player(0),
            CorePlayer::SecondAttackPlayer => Netplayer::Player(1),
            CorePlayer::None => Netplayer::Unknown,
            CorePlayer::All => Netplayer::Unknown,
            CorePlayer::Rule => Netplayer::Unknown,
        } 
    }

    pub fn calculate_replay(&self) -> Option<Replay> {
        let (host_player, client_player) = match self.first_attack_player? {
            PlayerIndex::Player1 => ( self.players[0].as_ref()?, self.players[1].as_ref()? ),
            PlayerIndex::Player2 => ( self.players[1].as_ref()?, self.players[0].as_ref()? ),
        };
        let seed_sequence = *self.duel.seed();
        let mut duel_options = DuelOptions::empty();
        if self.host_info.no_shuffle_deck {
            duel_options.insert(DuelOptions::PseudoShuffle);
        }
        let host_deck = host_player.deck.clone().into();
        let client_deck = client_player.deck.clone().into();
        let datas: Vec<ReplayData> = self.client_responses.iter().map(|response| ReplayData {
            data: response.response.clone(),
        }).collect();
        let mut replay = Replay { 
            header: ReplayHeader {
                id: ReplayVersion::V2 as u32,
                version: crate::PRO_VERSION as u32,
                flag: ReplayHeaderFlags::Uniform | ReplayHeaderFlags::Compressed,
                seed: 0,
                data_size: 0,
                start_time: self.start_time,
                props: [93, 0, 0, 128, 0, 0, 0, 0],
                seed_sequence,
                header_version: 1,
                reserved: [0; 3],
            }, 
            body: ReplayBody {
                host_name: host_player.name.clone(),
                client_name: client_player.name.clone(),
                tag_host_name: None,
                tag_client_name: None,
                start_lp: self.host_info.start_lp,
                start_hand: self.host_info.start_hand as u32,
                draw_count: self.host_info.draw_count as u32,
                duel_options,
                duel_rule: self.host_info.duel_rule as u16,
                host_deck,
                client_deck,
                tag_host_deck: None,
                tag_client_deck: None,
                datas,
            }
        };
        replay.fill_data_size();
        Some(replay)
    }

    pub fn win_and_end(&mut self, loser: CorePlayer, reason: WinReason) {
        let winner = loser.opponent();
        let win_message = gm::Message::Win(gm::Win { winner, reason });
        self.send(stoc::GameMessage { message: win_message }.into(), SendTarget::All);
        let winner_netplayer = self.to_player_index(winner);
        self.duel_winner.push(winner_netplayer);
        let current_decider = self.first_attack_decider.unwrap_or(PlayerIndex::Player1);
        self.first_attack_decider = Some(self.to_player_index(loser).unwrap_or(current_decider.opponent()));
        self.duel_end();
    }

    fn should_match_end(&self) -> bool {
        let mut end_count = self.configuration.override_best_of as usize;
        if end_count == 0 { end_count = if self.host_info.mode == Mode::Match { 3 } else { 1 }; }
        let end_win_count = (end_count + 1) / 2;
        let mut player_wins = [0, 0];
        for winner in &self.duel_winner {
            match winner {
                Some(PlayerIndex::Player1) => player_wins[0] += 1,
                Some(PlayerIndex::Player2) => player_wins[1] += 1,
                None => (),
            }
        }
        self.duel_winner.len() >= end_count || player_wins[0] >= end_win_count || player_wins[1] >= end_win_count || self.match_kill_card_code > 0
    }

    pub fn duel_end(&mut self) {
        if let Some(timer_task) = self.timer_task.take() {
            timer_task.abort();
        }
        if let Some(replay) = self.calculate_replay() {
            self.send(stoc::Replay{ replay: Box::new(replay) }.into(), SendTarget::All);
        }
        self.duel.end();
        self.duel_count += 1;
        if self.should_match_end() {
            self.stage = DuelStage::End;
            self.send(stoc::DuelEnd.into(), SendTarget::All);
        } else {
            for i in [0,1] {
                if let Some(player) = self.players[i].as_mut() { 
                    player.state = Some(ctos::MessageType::UpdateDeck);
                    player.ready = false;
                }
            }
            self.first_attack_player = None;
            self.stage = DuelStage::Siding;
            self.send(stoc::ChangeSide.into(), SendTarget::AllPlayer);
            self.send(stoc::WaitingSide.into(), SendTarget::AllObserver);
            self.end();
            self.duel.duel = ygopro_core_wrapper::Duel::new(self.configuration.seed(self.duel_count));
            // self.messages.clear();
            // self.masked_messages.clear();
            self.client_responses.clear();
            self.deck_reversed = false;
        }
    }

    fn send(&mut self, message: stoc::Message, target: SendTarget) {
        // Attention: we already serialize here.
        let message = Complex::from_message(message);
        if self.stage > DuelStage::Begin && !matches!(target, SendTarget::Single(_) | SendTarget::AllPlayer) {
            self.messages.push(message.clone());
            self.masked_messages.push(message.clone());
        }
        self._send(message, target);
    }

    fn _send_netplayer(&self, message: Complex<stoc::Message>, target: Netplayer) {
        match target {
            Netplayer::Player(index) => {
                if let Some(player) = &self.players[index as usize] {
                    player.stoc_sender.send(message).ok();
                }
            }
            Netplayer::Observer(index) => {
                if let Some(Some(observer)) = &self.observers.get(index as usize) {
                    observer.stoc_sender.send(message).ok();
                }
            }
            Netplayer::Unknown => warn!("Try to send message to an unknown position.")
        }
    }

    fn _send(&self, message: Complex<stoc::Message>, target: SendTarget) {
        match target {
            SendTarget::Single(netplayer) => self._send_netplayer(message, netplayer),
            SendTarget::Except(netplayer) => {
                // I believe we never need except an observer. If that happens, we can fix it in future.
                match netplayer {
                    Netplayer::Player(0) => self._send_netplayer(message.clone(), Netplayer::Player(1).into()),
                    Netplayer::Player(1) => self._send_netplayer(message.clone(), Netplayer::Player(0).into()),
                    _ => self._send(message.clone(), SendTarget::AllPlayer),
                }
                self._send(message, SendTarget::AllObserver);
            }
            SendTarget::All => {
                self._send(message.clone(), SendTarget::AllPlayer);
                self._send(message,         SendTarget::AllObserver);
            }
            SendTarget::AllPlayer => {
                self._send(message.clone(), Netplayer::Player(0).into());
                self._send(message,         Netplayer::Player(1).into());
            }
            SendTarget::AllObserver => {
                for observer in &self.observers {
                    if let Some(observer) = observer {
                        observer.stoc_sender.send(message.clone()).ok();
                    }
                }
            }
            SendTarget::None => {}
        }
    }

    pub fn send_game_message(&mut self, message: gm::Message, target: SendTarget) {
        let is_waiting_for = message.waiting_for();
        let can_player_0_see_unmasked = !message.should_mask(self.to_core_player(PlayerIndex::Player1));
        let can_player_1_see_unmasked = !message.should_mask(self.to_core_player(PlayerIndex::Player2));
        if is_waiting_for.is_some() { self.last_select_message = Some(message.clone()) }
        let masked_message = if self.configuration.no_mask { message.clone() } else { message.clone_masked() };
        let message = Complex::from_message(stoc::Message::GameMessage(stoc::GameMessage { message }));
        let masked_message = Complex::from_message(stoc::Message::GameMessage(stoc::GameMessage { message: masked_message }));
        // Select message always skip record steps.
        if is_waiting_for.is_none() {
            self.messages.push(message.clone());
            self.masked_messages.push(masked_message.clone());
        }
        self._send_game_message(message, masked_message, [can_player_0_see_unmasked, can_player_1_see_unmasked], target);
    }

    pub fn _send_game_message(&mut self, message: Complex<stoc::Message>, masked_message: Complex<stoc::Message>, can_player_see_unmasked: [bool; 2], target: SendTarget) {
        match target {
            SendTarget::Single(netplayer) => {
                match netplayer {
                    Netplayer::Player(index) => {
                        let can_see_unmasked = if index > 2 { false } else { can_player_see_unmasked[index as usize] };
                        self._send(if can_see_unmasked { message } else { masked_message }, target)
                    },
                    Netplayer::Observer(_) => self._send(masked_message, target),
                    Netplayer::Unknown => return,
                }
            },
            SendTarget::Except(netplayer) => {
                match netplayer {
                    Netplayer::Player(0) => { self._send_game_message(message.clone(), masked_message.clone(), can_player_see_unmasked, Netplayer::Player(1).into()); },
                    Netplayer::Player(1) => { self._send_game_message(message.clone(), masked_message.clone(), can_player_see_unmasked, Netplayer::Player(0).into()); },
                    _ => self._send_game_message(message.clone(), masked_message.clone(), can_player_see_unmasked, SendTarget::AllPlayer)
                }
                self._send_game_message(message, masked_message, can_player_see_unmasked, SendTarget::AllObserver);
            },
            SendTarget::All => {
                self._send_game_message(message.clone(), masked_message.clone(), can_player_see_unmasked, SendTarget::AllPlayer);
                self._send_game_message(message, masked_message, can_player_see_unmasked, SendTarget::AllObserver);
            },
            SendTarget::AllPlayer => {
                self._send_game_message(message.clone(), masked_message.clone(), can_player_see_unmasked, SendTarget::Single(Netplayer::Player(0)));
                self._send_game_message(message, masked_message, can_player_see_unmasked, SendTarget::Single(Netplayer::Player(1)));
            },
            SendTarget::AllObserver => self._send(masked_message, SendTarget::AllObserver),
            SendTarget::None => return,
        }
    }

    pub fn refresh(&mut self, player: CorePlayer, locations: Location, sequence: i8, query: Query) {
        if sequence >= 0 {
            let message = self.duel.refresh_card(&mut self.core_request_buffer, player, locations, sequence, query);
            self.send_game_message(message, SendTarget::All);
        } else {
            for message in self.duel.refresh_location(&mut self.core_request_buffer, player, locations, query).into_iter() {
                self.send_game_message(message, SendTarget::All);
            }
        }
    }


    pub fn start_timer(&mut self) {
        let sender = self.request_sender.clone();
        self.timer_task = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                if sender.send(Request::TimerTick).is_err() { break; }
            }
        }));
    }
    
    pub fn shuffle_deck(&mut self) {
        let first_attacker = self.first_attack_player.unwrap_or(PlayerIndex::Player1);
        let shuffle_order = [first_attacker as usize, first_attacker.opponent() as usize];
        for index in shuffle_order {
            if let Some(deck) = self.players[index].as_mut().map(|p| &mut p.deck) {
                self.duel.shuffle_deck(&mut deck.main);
            }
        }
    }
}

impl Deref for SingleDuel {
    type Target = common::Duel;
    fn deref(&self) -> &Self::Target { &self.duel }
}

impl DerefMut for SingleDuel {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.duel }
}

impl AsRef<common::Duel> for SingleDuel {
    fn as_ref(&self) -> &common::Duel { &self.duel }
}

impl AsMut<common::Duel> for SingleDuel {
    fn as_mut(&mut self) -> &mut common::Duel { &mut self.duel }
}

impl FromRequest<common::Request, State<SingleDuel>, Response> for &mut SingleDuel {
    fn from_request(bundle: &mut Bundle<common::Request, State<SingleDuel>, Response>) -> Option<Self> {
        Some(unsafe { &mut *(&mut bundle.state.duel as *mut SingleDuel) })
    }
}

pub struct SingleDuelHost {
    ctos_sender: mpsc::UnboundedSender<Request>,
}

impl SingleDuelHost {
    pub fn new(host_info: HostInfo, configuration: Configuration) -> (Self, tokio::task::JoinHandle<()>) {
        let single_duel = SingleDuel::new(host_info, configuration);
        let request_sender = single_duel.request_sender.clone();
        let handle = single_duel.run().expect("duel already started");
        (Self { ctos_sender: request_sender }, handle)
    }
}

impl RoomProvider<ctos::Message, Complex<stoc::Message>> for SingleDuelHost {
    type ServerToClientStream = UnboundedReceiverStream<Complex<stoc::Message>>;

    fn add(&mut self, client_to_server_stream: impl Stream<Item = ctos::Message> + Unpin + Send + 'static) -> Self::ServerToClientStream {
        let ctos_sender = self.ctos_sender.clone();
        let (stoc_sender, stoc_receiver) = mpsc::unbounded_channel();
        let (return_sender, return_receiver) = mpsc::unbounded_channel();
        ctos_sender.send(Request::Join { stoc_sender }).ok();
        
        tokio::spawn(async move {
            let mut ctos_stream = Box::pin(client_to_server_stream);
            let mut stoc_stream = UnboundedReceiverStream::new(stoc_receiver);
            let mut my_position: Netplayer = Netplayer::Unknown;
            loop {
                tokio::select! {
                    message = ctos_stream.next() => {
                        let gone = message.is_none();
                        let message = match message {
                            Some(message) => message,
                            None => ctos::Message::LeaveGame(ctos::LeaveGame)
                        };
                        ctos_sender.send(Request::Message(common::Request { message, extra: my_position })).ok();
                        if gone { break }
                    }
                    message = stoc_stream.next() => {
                        if let Some(message) = message {
                            match message.deref() {
                                stoc::Message::TypeChange(type_change) => my_position = type_change.player,
                                stoc::Message::LeaveGame(leave_game) => if leave_game.pos == my_position { break },
                                _ => ()
                            };
                            return_sender.send(message).ok();
                        } else {
                            break;
                        }
                    }
                }
            }
        });
        UnboundedReceiverStream::new(return_receiver)
    }
} 

mod ygopro_handlers {
    use std::io::Cursor;
    use std::sync::Arc;
    use std::sync::OnceLock;

    use arc_swap::ArcSwap;
    use binrw::BinRead;
    use binrw::BinWrite;
    use linkme::distributed_slice;

    use log::warn;
    use ygopro_data::constants::*;
    use ygopro_data::data::DuelOptions;
    
    use ygopro_data::data::QueryData;
    use ygopro_data::data::UpdateCardInfo;
    use ygopro_data::message::gm::GameMessage;
    use ygopro_data::message::{ctos, stoc, gm};
    use ygopro_derive::handler;
    use ygopro_derive::register_to;
    use ygopro_handler::Bundle;
    use ygopro_handler::Processor;

    use crate::common;
    use crate::common::Response;
    use crate::common::SendTarget;
    use crate::common::response_is_meaningful;
    use crate::managers::*;
    use crate::single_duel::PlayerIndex;
    use crate::single_duel::SingleDuel;

    pub type Request = common::Request;
    pub type State = common::State<SingleDuel>;
    pub type Handler = common::Handler<SingleDuel>;

    impl ygopro_handler::FromRequest<Request, State, Response> for &mut common::Request {
        fn from_request(bundle: &mut Bundle<Request, State, Response>) -> Option<Self> {
            Some(unsafe { &mut *(&mut bundle.request as *mut common::Request) })
        }
    }

    #[distributed_slice]
    pub static YGOPRO_HANDLERS: [fn() -> (u8, Handler)];
    pub static YGOPRO_PROCESSOR: OnceLock<ArcSwap<Processor<u8, Request, State, Response, Handler>>> = OnceLock::new();
    pub fn reset_processor() -> &'static ArcSwap<Processor<u8, Request, State, Response, Handler>> {
        YGOPRO_PROCESSOR.get_or_init(|| {
            let mut processor = Processor::new();
            for build in YGOPRO_HANDLERS.iter() {
                let (key, handler) = build();
                processor.register(key, handler);
            }
            processor.resolve();
            ArcSwap::from(Arc::new(processor))
        })
    }

    #[handler(ctos::Response)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_response(duel: &mut SingleDuel, player: PlayerIndex, response: &ctos::Response) {
        duel.client_responses.push(response.clone());
        {
            duel.response_buffer.fill(0);
            let mut cursor = Cursor::new(&mut duel.response_buffer[..]);
            response.write_le(&mut cursor).ok();
        }
        duel.set_responseb(&duel.response_buffer);
        if let Some(duel_player) = duel.get_player_mut_index(player) {
            duel_player.state = Some(ctos::MessageType::LeaveGame);
        }
        if duel.host_info.time_limit > 0 {
            let time_elapsed = duel.time_elapsed;
            duel.time_elapsed = 0;
            if let Some(duel_player) = duel.get_player_mut_index(player) {
                duel_player.time_limit = duel_player.time_limit.saturating_sub(time_elapsed);
            }
        }
        if let Some(last_select_message) = &duel.last_select_message && response_is_meaningful(&response.response, last_select_message) {
            let add_time = duel.configuration.add_time_after_operation;
            let add_deposit = duel.configuration.add_small_time_deposit_after_operation;
            let time_limit = duel.host_info.time_limit;
            if let Some(duel_player) = duel.get_player_mut_index(player) {
                if duel_player.time_backed > 0 && duel_player.time_limit < time_limit {
                    duel_player.time_limit = duel_player.time_limit.saturating_add(add_time);
                    duel_player.time_compensator = duel_player.time_compensator.saturating_add(add_deposit);
                    duel_player.time_backed = duel_player.time_backed.saturating_sub(add_time);
                }
            }
        }
        duel.request_sender.send(super::Request::Evolve).ok();
    }

    #[handler(ctos::HandResult)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_hand_result(duel: &mut SingleDuel, player: PlayerIndex, hand_result: &ctos::HandResult) {
        if let Some(duel_player) = duel.get_player_mut_index(player) {
            duel_player.hand = Some(hand_result.res);
        }
        let (message, winner) = {
            let (player1, player2) = duel.players.split_at_mut(1);
            let player1 = match player1[0].as_mut() { Some(p) => p, None => return };
            let player2 = match player2[0].as_mut() { Some(p) => p, None => return };
            let hand1 = match player1.hand { Some(res) => res, None => return };
            let hand2 = match player2.hand { Some(res) => res, None => return };
            let observer_message = stoc::HandResult { hand1, hand2 };
            let result = observer_message.judge();
            match result {
                HandResult::Draw => {
                    player1.hand = None;
                    player2.hand = None;
                    player1.state = Some(ctos::MessageType::HandResult);
                    player2.state = Some(ctos::MessageType::HandResult);
                    duel.send(stoc::SelectHand.into(), SendTarget::AllPlayer);
                    (observer_message, None)
                },
                HandResult::Win => {
                    player1.state = Some(ctos::MessageType::TpResult);
                    player2.state = Some(ctos::MessageType::LeaveGame);
                    (observer_message, Some(PlayerIndex::Player1))
                },
                HandResult::Lose => {
                    player1.state = Some(ctos::MessageType::LeaveGame);
                    player2.state = Some(ctos::MessageType::TpResult);
                    (observer_message, Some(PlayerIndex::Player2))
                }
            }
        };
        
        duel.send(message.swap_clone().into(), SendTarget::Single(Netplayer::Player(1)));
        duel.send(message.into(), SendTarget::Except(Netplayer::Player(1)));
        if let Some(winner) = winner {
            duel.first_attack_decider = Some(winner);
            duel.send(stoc::SelectTp.into(), SendTarget::Single(winner.into()));
            duel.stage = DuelStage::Firstgo;
        }
    }

    #[handler(ctos::TpResult)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_tp_result(duel: &mut SingleDuel, player: PlayerIndex, tp_result: &ctos::TpResult) {
        duel.stage = DuelStage::Dueling;
        duel.start_time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|r| r.as_secs() as u32)
                .unwrap_or(0);
        duel.first_attack_player = Some(if tp_result.result == CorePlayer::FirstAttackPlayer { player } else { player.opponent() });
        duel.set_player_info(CorePlayer::FirstAttackPlayer,  duel.host_info.start_lp as i32, duel.host_info.start_hand as i32, duel.host_info.draw_count as i32);
        duel.set_player_info(CorePlayer::SecondAttackPlayer, duel.host_info.start_lp as i32, duel.host_info.start_hand as i32, duel.host_info.draw_count as i32);
        for script in &duel.configuration.preloaded_scripts {
            if duel.preload_script(script) == 0 {
                log::debug!("Failed to preload script: {script}");
            }
        }
        if !(duel.host_info.no_shuffle_deck || duel.configuration.no_init_shuffle_deck) {
            duel.shuffle_deck();
        }
        let mut player1 = match duel.players[0].as_ref() { Some(p) => p, None => return };
        let mut player2 = match duel.players[1].as_ref() { Some(p) => p, None => return };
        if (tp_result.result == CorePlayer::FirstAttackPlayer && player == PlayerIndex::Player2)
            || (tp_result.result == CorePlayer::SecondAttackPlayer && player == PlayerIndex::Player1) {
            std::mem::swap(&mut player1, &mut player2);
        }
        for &code in player1.deck.main.iter().rev() {
            duel.new_card(code, CorePlayer::FirstAttackPlayer, CorePlayer::FirstAttackPlayer, Location::Deck, 0, Position::FacedownDefense);
    }
        for &code in player1.deck.extra.iter().rev() {
            duel.new_card(code, CorePlayer::FirstAttackPlayer, CorePlayer::FirstAttackPlayer, Location::Extra, 0, Position::FacedownDefense);
        }
        for &code in player2.deck.main.iter().rev() {
            duel.new_card(code, CorePlayer::SecondAttackPlayer, CorePlayer::SecondAttackPlayer, Location::Deck, 0, Position::FacedownDefense);
        }
        for &code in player2.deck.extra.iter().rev() {
            duel.new_card(code, CorePlayer::SecondAttackPlayer, CorePlayer::SecondAttackPlayer, Location::Extra, 0, Position::FacedownDefense);
        }
        let deck1 = duel.query_field_count(CorePlayer::FirstAttackPlayer, Location::Deck) as u16;
        let extra1 = duel.query_field_count(CorePlayer::FirstAttackPlayer, Location::Extra) as u16;
        let deck2 = duel.query_field_count(CorePlayer::SecondAttackPlayer, Location::Deck) as u16;
        let extra2 = duel.query_field_count(CorePlayer::SecondAttackPlayer, Location::Extra) as u16;
        let start_lp = duel.host_info.start_lp as i32;
        let duel_rule = duel.host_info.duel_rule;
        let start = |player_type: u8| gm::Message::Start(gm::Start {
            player_type,
            rule: duel_rule,
            player1_lp: start_lp,
            player2_lp: start_lp,
            player1_deck_count: deck1,
            player1_extra_count: extra1,
            player2_deck_count: deck2,
            player2_extra_count: extra2,
        });
        duel.send(stoc::GameMessage { message: start(0) }.into(), SendTarget::Single(duel.to_net_player(CorePlayer::FirstAttackPlayer)));
        duel.send(stoc::GameMessage { message: start(1) }.into(), SendTarget::Single(duel.to_net_player(CorePlayer::SecondAttackPlayer)));
        let observer_player_type = match duel.first_attack_player {
            Some(PlayerIndex::Player1) => 0x10,
            Some(PlayerIndex::Player2) => 0x11,
            _ => unreachable!(),
        };
        duel.send(stoc::GameMessage { message: start(observer_player_type) }.into(), SendTarget::AllObserver);
        duel.refresh(CorePlayer::All, Location::Extra, -1, Query::empty());
        let mut options = DuelOptions::empty();
        if duel.host_info.no_shuffle_deck { options.insert(DuelOptions::PseudoShuffle); }
        duel.start(options, duel.host_info.duel_rule);
        let time_limit = duel.host_info.time_limit;
        if time_limit > 0 { 
            duel.time_elapsed = 0;
            let (player1, player2) = duel.players.split_at_mut(1);
            let player1 = match player1[0].as_mut() { Some(p) => p, None => return };
            let player2 = match player2[0].as_mut() { Some(p) => p, None => return };
            player1.time_limit = time_limit;
            player2.time_limit = time_limit;
            player1.time_backed = if duel.configuration.max_add_time_each_turn == 0 { if duel.configuration.add_time_after_operation > 0 { time_limit } else { 0 } } else { duel.configuration.max_add_time_each_turn };
            player2.time_backed = if duel.configuration.max_add_time_each_turn == 0 { if duel.configuration.add_time_after_operation > 0 { time_limit } else { 0 } } else { duel.configuration.max_add_time_each_turn };
            player1.time_compensator = 0;
            player2.time_compensator = 0;
            duel.start_timer();
        }
        duel.request_sender.send(super::Request::Evolve).ok();
    }

    #[handler(ctos::UpdateDeck)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_update_deck(duel: &mut SingleDuel, player: PlayerIndex, update_deck: &ctos::UpdateDeck) -> Option<stoc::Message> {
        let netplayer: Netplayer = player.into();
        if duel.get_player_index(player)?.ready {
            warn!("UpdateDeck requested but player is already ready");
            return None;
        }
        let mut deck = update_deck.deck.clone();
        if duel.duel_count == 0 {
            let data_manager = data_manager::load().clone().expect("unintied data manager");
            let player = duel.get_player_mut_index(player)?;
            player.deck_error = deck.load(|code| data_manager.get_card(code));
            player.deck = deck;
        } else {
            let data_manager = data_manager::load().clone().expect("unintied data manager");
            let side_check_result = duel.get_player_index(player)?.deck.check_after_replacing_side(&mut deck, |code| data_manager.get_card(code));
            if let Err(_error) = side_check_result {
                return Some(stoc::ErrorMessage { err: ErrorMessage::SideError }.into());
            }
            if let Some(player) = duel.get_player_mut_index(player) {
                player.deck = deck;
                player.ready = true;
            }
            duel.send(stoc::DuelStart.into(), netplayer.into());
            let ready = {
                let player1 = duel.players[0].as_ref()?;
                let player2 = duel.players[1].as_ref()?;
                player1.ready && player2.ready
            };
            if ready {
                let decider = duel.first_attack_decider.unwrap_or(PlayerIndex::Player1);
                duel.send(stoc::SelectTp.into(), decider.into());
                duel.get_player_mut_index(decider)?.state = Some(ctos::MessageType::TpResult);
                duel.get_player_mut_index(decider.opponent())?.state = Some(ctos::MessageType::LeaveGame);
                duel.stage = DuelStage::Firstgo; 
            }
        }
        None
    }

    #[handler(ctos::CreateGame)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_create_game(duel: &mut SingleDuel, create_game: &ctos::CreateGame) {
        duel.host_info = create_game.info.clone();
        duel.name = create_game.name.clone();
        duel.pass = create_game.pass.clone();
    }

    #[handler(ctos::JoinGame)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_join_game(duel: &mut SingleDuel, request: &mut common::Request, join_game: &ctos::JoinGame) -> Result<Vec<stoc::Message>, stoc::Message> {
        if join_game.version != crate::PRO_VERSION {
            return Err(stoc::ErrorMessage { err: ErrorMessage::VersionError(crate::PRO_VERSION) }.into());
        }
        if !duel.pass.is_empty() && join_game.pass != duel.pass {
            return Err(stoc::ErrorMessage { err: ErrorMessage::JoinError(JoinError::WrongPassword) }.into());
        }
        if duel.last_init_player.is_none() {
            return Ok(vec![])
        }
        let mut response_messages = vec![];

        // calculate current user position
        let is_creator = duel.players[0].is_none() && duel.players[1].is_none() && duel.observers.is_empty();
        let mut observer_count = duel.observer_count();
        let pos = if duel.players[0].is_none() {
            Netplayer::Player(0)
        } else if duel.players[1].is_none() {
            Netplayer::Player(1)
        } else {
            let observer_index = duel.observers.iter().position(|v| v.is_none()).unwrap_or(duel.observers.len()) as u8;
            observer_count = observer_count + 1;
            Netplayer::Observer(observer_index)
        };
        request.extra = pos;
        if is_creator { duel.host_player = pos; }
 
        let deck_manager = deck_manager::load();
        let mut join_info = duel.host_info.clone();
        if let Some(lflist) = deck_manager.as_ref().and_then(|dm| dm.get_lflist_by_index(join_info.lflist)) {
            join_info.lflist = lflist.hash;
        }
        response_messages.push(stoc::JoinGame{ info: join_info }.into());
        response_messages.push(stoc::TypeChange{ 
            player: pos,
            host: is_creator
        }.into());
        
        // broadcast player change
        let player = duel.last_init_player.take().expect("cannot get init player when join game but we just checked");
        if matches!(pos, Netplayer::Observer(_)) {
            duel.send(stoc::HsWatchChange { watch_count: observer_count }.into(), SendTarget::All);
        } else {
            duel.send(stoc::HsPlayerEnter { name: player.name.clone(), pos }.into(), SendTarget::All);
        }

        // actual player change
        match pos {
            Netplayer::Observer(index) => {
                if index as usize >= duel.observers.len() { duel.observers.push(Some(player)); } 
                else { duel.observers[index as usize] = Some(player); }
            }
            Netplayer::Player(0) => { duel.players[0] = Some(player.into()); }
            Netplayer::Player(1) => { duel.players[1] = Some(player.into()); }
            _ => warn!("try to put into an illegal player pos")
        };

        // tell current user now how room is now.
        for i in [0u8, 1u8] {
            if let Some(player) = duel.players[i as usize].as_ref() {
                response_messages.push(stoc::HsPlayerEnter { name: player.name.clone(), pos: Netplayer::Player(i) }.into());
                if player.ready { response_messages.push(stoc::HsPlayerChange { status: PlayerChange::new()
                    .with_player(Netplayer::Player(i))
                    .with_state(PlayerChangeState::Ready)
                }.into()); }
            }
        };
        if observer_count > 0 {
            response_messages.push(stoc::HsWatchChange{ watch_count: observer_count }.into());
        }
        if duel.stage > DuelStage::Begin && matches!(pos, Netplayer::Observer(_)) {
            duel.request_sender.send(crate::single_duel::Request::Soumatou(pos)).ok();
        }
        Ok(response_messages)
    }

    #[handler(ctos::HsToDuelist)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_hs_to_duelist(duel: &mut SingleDuel, request: &mut common::Request, player: Netplayer) -> Option<stoc::Message> {
        let observer_index = if let Netplayer::Observer(observer_index) = player {
            observer_index as usize
        } else {
            warn!("HsToDuelist requested by non-observer");
            return None;
        };
        if duel.players[0].is_some() && duel.players[1].is_some() {
            warn!("HsToDuelist requested but both player slots are full");
            return None;
        }
        let Some(observer) = duel.observers[observer_index].take() else {
            warn!("try to convert observer to player but observer dont exist");
            return None;
        };
        let i_am_host = duel.host_player == player;
        let new_position_index = if duel.players[0].is_none() { 0 } else { 1 };
        let new_position = Netplayer::Player(new_position_index as u8);
        request.extra = new_position;
        if i_am_host { duel.host_player = new_position; }
        let name = observer.name.clone();
        duel.players[new_position_index] = Some(observer.into());
        duel.send(stoc::HsPlayerEnter { name, pos: new_position }.into(), SendTarget::All);
        duel.send(stoc::HsWatchChange { watch_count: duel.observer_count() }.into(), SendTarget::All);
        Some(stoc::TypeChange {
            player: new_position,
            host: i_am_host
        }.into())
    }

    #[handler(ctos::HsToObserver)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_hs_to_observer(duel: &mut SingleDuel, request: &mut common::Request, player: PlayerIndex) -> Option<stoc::Message> {
        let original_netplayer: Netplayer = player.into();
        let position = player as u8 as usize;
        let Some(duel_player) = duel.players[position].take() else {
            warn!("to_observer requested but player slot is empty");
            return None;
        };
        let current_netplayer = duel.insert_observer(duel_player.player);
        request.extra = current_netplayer;
        let i_am_host = duel.host_player == original_netplayer;
        if i_am_host { duel.host_player = current_netplayer }
        duel.send(stoc::HsPlayerChange { 
            status: PlayerChange::new()
                .with_state(PlayerChangeState::Observe)
                .with_player(original_netplayer) 
        }.into(), SendTarget::All);
        Some(stoc::TypeChange {
            player: current_netplayer,
            host: i_am_host
        }.into())
    }

    #[handler(ctos::LeaveGame)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_leave_game(duel: &mut SingleDuel, player: Netplayer) -> bool {
        if player == duel.host_player {
            let new_host: Netplayer = if duel.players[0].is_some() && player != Netplayer::Player(0) {
                Netplayer::Player(0)
            } else if duel.players[1].is_some() && player != Netplayer::Player(1) {
                Netplayer::Player(1)
            } else {
                duel.end();
                return true;
            };
            duel.host_player = new_host;
            if duel.stage == DuelStage::Begin {
                if let Some(player) = duel.get_player_mut(new_host) {
                    player.ready = false;
                }
                duel.send(stoc::TypeChange {
                    player: new_host,
                    host: true
                }.into(), SendTarget::Single(new_host));
            }
        }

        match player {
            Netplayer::Observer(observer_index) => {
                let index = observer_index as usize;
                if index == 255 {
                    warn!("LeaveGame requested by unknown observer");
                } else {
                    duel.observers[index] = None;
                    while duel.observers.last().is_none() {
                        duel.observers.pop();
                    }
                    if duel.stage == DuelStage::Begin {
                        let observer_count = duel.observer_count();
                        duel.send(stoc::HsWatchChange { watch_count: observer_count }.into(), SendTarget::All);
                    }
                }
            }
            Netplayer::Player(leaving_netplayer) => {
                if duel.stage == DuelStage::Begin {
                    duel.players[leaving_netplayer as usize] = None;
                    let leave_message: stoc::Message = stoc::HsPlayerChange { status: PlayerChange::new()
                        .with_state(PlayerChangeState::Leave)
                        .with_player(player)
                    }.into();
                    duel.send(leave_message, SendTarget::All);
                } else {
                    if duel.stage == DuelStage::Siding {
                        duel.send(stoc::DuelStart.into(), SendTarget::AllPlayer);
                    }
                    if duel.stage != DuelStage::End {
                        let leaving_index = if leaving_netplayer == 0 { PlayerIndex::Player1 } else { PlayerIndex::Player2 };
                        let loser = duel.to_core_player(leaving_index);
                        duel.win_and_end(loser, WinReason::OpponentLeave);
                        return true;
                    }
                }
                duel.players[leaving_netplayer as usize] = None;
            }
            Netplayer::Unknown => {}
        }
        match duel.configuration.terminate_when {
            SendTarget::Single(netplayer) => {
                match netplayer {
                    Netplayer::Player(index) => duel.players.get(index as usize).map_or(true, Option::is_none),
                    Netplayer::Observer(index) => duel.observers.get(index as usize).map_or(true, Option::is_none),
                    Netplayer::Unknown => { warn!("set terminate condition to unknown player"); duel.players[0].is_none() && duel.players[1].is_none() && duel.observers.is_empty() },
                }
            },
            SendTarget::Except(_) => { warn!("set terminate condition to not supported except"); duel.players[0].is_none() && duel.players[1].is_none() && duel.observers.is_empty() },
            SendTarget::All => duel.players[0].is_none() && duel.players[1].is_none() && duel.observers.is_empty(),
            SendTarget::AllPlayer => duel.players[0].is_none() && duel.players[1].is_none(),
            SendTarget::AllObserver => duel.observers.is_empty(),
            SendTarget::None => { warn!("a room is set to terminate in no case. this mean this room will be eternal."); false },
        }
    }

    #[handler(ctos::HsStart)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_hs_start(duel: &mut SingleDuel, player: Netplayer) {
        if player != duel.host_player {
            warn!("HsStart requested by non-host");
            return;
        }

        let (deck1_main, deck1_side, deck1_extra, deck2_main, deck2_side, deck2_extra) = {
            let player1 = match duel.players[0].as_ref() { Some(p) => p, None => { warn!("HsStart: player1 missing"); return; } };
            let player2 = match duel.players[1].as_ref() { Some(p) => p, None => { warn!("HsStart: player2 missing"); return; } };
            if !player1.ready || !player2.ready {
                warn!("HsStart: not all players ready");
                return;
            }
            (
                player1.deck.main.len() as u16,
                player1.deck.side.len() as u16,
                player1.deck.extra.len() as u16,
                player2.deck.main.len() as u16,
                player2.deck.side.len() as u16,
                player2.deck.extra.len() as u16,
            )
        };

        duel.stage = DuelStage::Finger;
        duel.send(stoc::DuelStart.into(), SendTarget::All);

        let player1_count = stoc::DeckCount {
            mainc_s: deck1_main, sidec_s: deck1_side, extrac_s: deck1_extra,
            mainc_o: deck2_main, sidec_o: deck2_side, extrac_o: deck2_extra,
        };
        let player2_count = stoc::DeckCount {
            mainc_s: deck2_main, sidec_s: deck2_side, extrac_s: deck2_extra,
            mainc_o: deck1_main, sidec_o: deck1_side, extrac_o: deck1_extra,
        };
        duel.send(player1_count.into(), SendTarget::Single(Netplayer::Player(0)));
        duel.send(player2_count.into(), SendTarget::Single(Netplayer::Player(1)));

        duel.send(stoc::SelectHand.into(), SendTarget::AllPlayer);

        let (player1, player2) = duel.players.split_at_mut(1);
        if let (Some(player1), Some(player2)) = (player1[0].as_mut(), player2[0].as_mut()) {
            player1.hand = None;
            player2.hand = None;
            player1.state = Some(ctos::MessageType::HandResult);
            player2.state = Some(ctos::MessageType::HandResult);
        } else {
            warn!("HsStart: one of the players is missing");
        }
    }

    #[handler(ctos::Surrender)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_surrender(duel: &mut SingleDuel, index: PlayerIndex) {
        if duel.stage != DuelStage::Dueling {
            warn!("Surrender requested but not in dueling stage");
            return;
        }
        let core_surrendering = duel.to_core_player(index);
        duel.win_and_end(core_surrendering, WinReason::OpponentSurrender);
    }

    #[handler(ctos::TimeConfirm)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_time_confirm(duel: &mut SingleDuel, index: PlayerIndex) {
        if duel.host_info.time_limit == 0 { return; }
        if Some(index) != duel.last_response {
            warn!("TimeConfirm requested by wrong player");
            return;
        }
        let ignore_duration = duel.configuration.ignore_small_time_under_this_duration;
        let time_elapsed = duel.time_elapsed;
        let Some(duel_player) = duel.get_player_mut_index(index) else {
            warn!("TimeConfirm requested but player slot is empty");
            return;
        };
        duel_player.state = Some(ctos::MessageType::Response);
        if time_elapsed < ignore_duration && time_elapsed <= duel_player.time_compensator {
            duel_player.time_compensator -= time_elapsed;
        } else {
            duel_player.time_limit = duel_player.time_limit.saturating_sub(time_elapsed);
        }
        duel.time_elapsed = 0;
    }

    #[handler(ctos::Chat)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_chat(duel: &mut SingleDuel, player: Netplayer, chat: &ctos::Chat) {
        let chat = stoc::Chat {
            player: player.into(),
            msg: chat.msg.clone()
        };
        duel.send(chat.into(), SendTarget::All);
    }

    #[handler(ctos::PlayerInfo)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_player_info(duel: &mut SingleDuel, player_info: &ctos::PlayerInfo) {
        if let Some(player) = duel.last_init_player.as_mut() {
            player.name = player_info.name.clone();
        } else {
            warn!("We receive a player_info, but no user is waiiting init.");
        }
    }

    #[handler(ctos::HsReady)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_hs_ready(duel: &mut SingleDuel, index: PlayerIndex) -> Vec<stoc::Message> {
        let netplayer: Netplayer = index.into();
        if duel.stage != DuelStage::Begin {
            warn!("HsReady requested outside Begin stage");
            return vec![];
        }
        let no_check_deck = duel.host_info.no_check_deck;
        let lflist_hash = duel.host_info.lflist;
        let rule = duel.host_info.rule;
        let Some(duel_player) = duel.get_player_mut_index(index) else {
            warn!("HsReady requested by non-player");
            return vec![];
        };
        if duel_player.ready {
            warn!("HsReady requested but player is already ready");
            return vec![];
        }
        let mut messages = vec![];
        if !no_check_deck {
            let deck_manager = deck_manager::load();
            let data_manager = data_manager::load();
            let data_manager = data_manager.as_ref().expect("unintied data manager");
            let lflist = deck_manager.as_ref().and_then(|dm| dm.get_lflist_by_hash(lflist_hash)).cloned().unwrap_or_else(|| ygopro_data::data::LFList::new(String::new()));
            if let Some(deck_error) = duel_player.deck_error.take() {
                messages.push(stoc::ErrorMessage { err: ErrorMessage::DeckError(deck_error) }.into());
            }
            if messages.is_empty() && let Err(deck_error) = duel_player.deck.prepare(&lflist, rule, |code| data_manager.get_card(code)) {
                messages.push(stoc::ErrorMessage { err: ErrorMessage::DeckError(deck_error) }.into());
            }
            if !messages.is_empty() {
                messages.insert(0, stoc::HsPlayerChange { status: PlayerChange::new().with_state(PlayerChangeState::Notready).with_player(netplayer)}.into());
            }
        }
        if messages.is_empty() {
            duel_player.ready = true;
            duel.send(stoc::HsPlayerChange { status: PlayerChange::new().with_state(PlayerChangeState::Ready).with_player(netplayer)}.into(), SendTarget::All);
        }
        messages
    }

    #[handler(ctos::HsNotReady)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_hs_not_ready(duel: &mut SingleDuel, index: PlayerIndex) {
        if duel.stage != DuelStage::Begin { 
            warn!("HsNotReady requested outside Begin stage"); 
            return; 
        }
        let Some(duel_player) = duel.get_player_mut_index(index) else {
            warn!("HsNotReady requested by non-player");
            return;
        };
        if !duel_player.ready { 
            warn!("HsNotReady requested but player is already not ready"); 
            return 
        }
        duel_player.ready = false;
        let netplayer: Netplayer = index.into();
        duel.send(stoc::HsPlayerChange { 
            status: PlayerChange::new()
                .with_state(PlayerChangeState::Notready)
                .with_player(netplayer) 
        }.into(), SendTarget::All);
    }

    #[handler(ctos::HsKick)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_hs_kick(duel: &mut SingleDuel, kicker: Netplayer, kick: &ctos::HsKick) {
        if kicker != duel.host_player {
            warn!("HsKick requested by non-host");
            return;
        }
        if duel.stage != DuelStage::Begin {
            warn!("HsKick requested outside Begin stage");
            return;
        }
        let Netplayer::Player(target) = kick.pos else {
            warn!("HsKick requested to kick non-player");
            return;
        };
        if kicker == kick.pos {
            warn!("HsKick: cannot kick self");
            return;
        }
        if duel.players[target as usize].is_none() {
            warn!("HsKick: target slot empty");
            return;
        }
        duel.players[target as usize] = None;
        duel.send(stoc::HsPlayerChange {
            status: PlayerChange::new()
                .with_state(PlayerChangeState::Leave)
                .with_player(kick.pos)
        }.into(), SendTarget::All);
    }

    #[handler(ctos::RequestField)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_request_field(duel: &mut SingleDuel, player: PlayerIndex) -> Vec<stoc::Message> {
        let mut messages = vec![];
        messages.push(stoc::DuelStart.into());

        let player_type: u8 = duel.to_core_player(player) as u8;
        let start_lp = duel.host_info.start_lp as i32;
        messages.push(stoc::GameMessage {
            message: gm::Message::Start(gm::Start {
                player_type,
                rule: duel.host_info.duel_rule,
                player1_lp: start_lp,
                player2_lp: start_lp,
                player1_deck_count: 0,
                player1_extra_count: 0,
                player2_deck_count: 0,
                player2_extra_count: 0,
            })
        }.into());

        messages.push(stoc::GameMessage { message: gm::Message::NewTurn(gm::NewTurn { player: CorePlayer::FirstAttackPlayer }) }.into());
        if duel.turn_player == CorePlayer::SecondAttackPlayer {
            messages.push(stoc::GameMessage { message: gm::Message::NewTurn(gm::NewTurn { player: CorePlayer::SecondAttackPlayer }) }.into());
        }

        messages.push(stoc::GameMessage {
            message: gm::Message::NewPhase(gm::NewPhase {
                phase: duel.phase,
            })
        }.into());

        let len = duel.duel.query_field_info(&mut duel.core_request_buffer);
        let mut cursor = Cursor::new(&duel.core_request_buffer[..len as usize]);
        let message = gm::Message::read_le(&mut cursor).unwrap();
        messages.push(stoc::GameMessage { message }.into());


        let core_player = duel.to_core_player(player);
        let opponent = core_player.opponent();
        for location in [Location::MZone, Location::SZone, Location::Hand, Location::Grave, Location::Extra, Location::Removed] {
            for mut gm_message in duel.duel.refresh_location(&mut duel.core_request_buffer, opponent, location, Query::all()) {
                if !duel.configuration.no_mask { gm_message.mask(); }
                messages.push(stoc::GameMessage { message: gm_message }.into());
            }
            for gm_message in duel.duel.refresh_location(&mut duel.core_request_buffer, core_player, location, Query::all()) {
                messages.push(stoc::GameMessage { message: gm_message }.into());
            }
        }

        if duel.deck_reversed {
            messages.push(stoc::GameMessage { message: gm::Message::ReverseDeck(gm::ReverseDeck) }.into());
        }

        for player in [CorePlayer::FirstAttackPlayer, CorePlayer::SecondAttackPlayer] {
            let message = duel.duel.query_location_cards(&mut duel.core_request_buffer, player, Location::Deck, Query::Code | Query::Position);
            let message = match message { gm::Message::UpdateData(update) => update, _ => continue };
            let data = match message.data.last() { Some(UpdateCardInfo::Data(data)) => data, _ => continue };
            let code = match &data[0] { QueryData::Code(code) => *code as u32, _ => continue };
            let position = match &data[1] { QueryData::Position(location) => location.position, _ => continue };
            let is_faceup = !position.is_face_down();
            if !duel.deck_reversed && !is_faceup { continue; }
            let message = gm::DeckTop {
                player,
                sequence: 0,
                code: gm::CardCode::new().with_id(code).with_is_public(is_faceup),
            }.into();
            messages.push(stoc::GameMessage { message }.into());
        }

        for player_index in [PlayerIndex::Player1, PlayerIndex::Player2] {
            let base = duel.get_player_index(player_index).map_or(0, |player| player.time_limit);
            let left_time = if Some(player_index) == duel.last_response {
                base.saturating_sub(duel.time_elapsed)
            } else {
                base
            };
            messages.push(stoc::TimeLimit {
                player: duel.to_core_player(player_index),
                left_time,
            }.into());
        }

        messages.push(stoc::FieldFinish.into());
        if let Some(message) = &duel.last_select_message && duel.last_response.unwrap_or(duel.to_player_index(CorePlayer::FirstAttackPlayer).unwrap_or(PlayerIndex::Player1)) == player {
           messages.push(stoc::GameMessage { message: message.clone() }.into());
        }
        messages
    }
}

mod ygocore_handlers {
    use std::io::Cursor;
    use std::sync::Arc;
    use std::sync::OnceLock;

    use arc_swap::ArcSwap;
    use binrw::BinRead;
    use linkme::distributed_slice;
    use log::warn;
    
    use ygopro_core_wrapper::ProcessResultFlags;
    use ygopro_data::constants::*;
    use ygopro_data::message::*;
    use ygopro_data::message::gm::GameMessage;
    use ygopro_handler::Bundle;
    use ygopro_handler::FromRequest;
    use ygopro_handler::IntoResponse;
    use ygopro_handler::Processor;
    use ygopro_derive::handler;
    use ygopro_derive::register_to;

    use crate::common;
    use crate::common::SendTarget;
    use crate::single_duel::SingleDuel;

    pub type Request = ygopro_handler::extract::Request<gm::Message, Netplayer>; 
    pub type State = common::State<SingleDuel>;
    pub type Handler = ygopro_handler::sync_handler::SyncHandler<Request, State, Response>;

    pub struct Response {
        pub target: SendTarget,
        pub refresh: (CorePlayer, Location, i8, Query)
    }

    impl Default for Response {
        fn default() -> Self {
            Self { target: SendTarget::All, refresh: (CorePlayer::None, Location::empty(), -1, Query::empty()) }
        }
    }

    impl FromRequest<Request, State, Response> for &mut SingleDuel {
        fn from_request(bundle: &mut Bundle<Request, State, Response>) -> Option<Self> {
            Some(unsafe { &mut *(&mut bundle.state.duel as *mut SingleDuel) })
        }
    }

    impl FromRequest<Request, State, Response> for &SingleDuel {
        fn from_request(bundle: &mut Bundle<Request, State, Response>) -> Option<Self> {
            Some(unsafe { &*(&bundle.state.duel as *const SingleDuel) })
        }
    }


    impl IntoResponse<Response> for () {
        fn into_response(self) -> Response {
            Default::default()
        }
    }

    impl IntoResponse<Response> for Netplayer {
        fn into_response(self) -> Response {
            Response { target: SendTarget::Single(self), refresh: (CorePlayer::None, Location::empty(), -1, Query::empty()) }
        }
    }


    impl IntoResponse<Response> for SendTarget {
        fn into_response(self) -> Response {
            Response { target: self, refresh: (CorePlayer::None, Location::empty(), -1, Query::empty()) }
        }
    }

    impl IntoResponse<Response> for (CorePlayer, Location) {
        fn into_response(self) -> Response {
            Response { target: SendTarget::All, refresh: (self.0, self.1, -1, Query::empty()) }
        }
    }

    impl IntoResponse<Response> for (CorePlayer, Location, i8) {
        fn into_response(self) -> Response {
            Response { target: SendTarget::All, refresh: (self.0, self.1, self.2, Query::empty()) }
        }
    }
    
    impl IntoResponse<Response> for (CorePlayer, Location, i8, Query) {
        fn into_response(self) -> Response {
            Response { target: SendTarget::All, refresh: self }
        }
    }

    #[distributed_slice]
    pub static YGOCORE_HANDLERS: [fn() -> (u8, Handler)];
    pub static YGOCORE_PROCESSOR: OnceLock<ArcSwap<Processor<u8, Request, common::State<SingleDuel>, Response, Handler>>> = OnceLock::new();
    pub fn reset_processor() -> &'static ArcSwap<Processor<u8, Request, common::State<SingleDuel>, Response, Handler>> {
        YGOCORE_PROCESSOR.get_or_init(|| {
            let mut processor = Processor::new();
            for build in YGOCORE_HANDLERS.iter() {
                let (key, handler) = build();
                processor.register(key, handler);
            }
            processor.resolve();
            ArcSwap::from(Arc::new(processor))
        })
    }

    /// process input messages, until waiting for user input or duel end.
    /// named `process` in original ygopro.
    pub fn evolve(duel: &mut SingleDuel) -> Vec<gm::Message> {
        let mut messages = vec![];
        loop {
            let result = duel.process();
            let engine_flag = result.flags();
            let engine_length = result.data_length() as usize;
            if engine_length > 0 {
                duel.duel.get_message(&mut duel.core_request_buffer[..]);
                let mut cursor = Cursor::new(&duel.core_request_buffer[..engine_length]);
                while let Ok(message) = gm::Message::read_le(&mut cursor) {
                    messages.push(message);
                }
            }
            if engine_flag == ProcessResultFlags::End { break; }
            // we should use engine_flag is Flags::Waiting to check if need continue.
            // but sadly, ygocore will incorrectly send waiting even need to continue.
            // so just like original ygopro do, we check specific message here.
            if messages.last().map_or(false, |m| m.waiting_for().is_some() || matches!(m, gm::Message::Retry(_))) { break; }
        }
        messages
    }

    pub fn set_waiting(duel: &mut SingleDuel, player: CorePlayer) -> Option<()> {
        let index = match duel.to_player_index(player) {
            Some(player) => player,
            None => {
                warn!("try to set waiting to a non-sense plyaer: {:?}", player);
                return None
            }
        };
        duel.last_response = Some(index);
        duel.send(
            stoc::Message::GameMessage(stoc::GameMessage { message: gm::Message::Waiting(gm::Waiting) }),
            SendTarget::Single(index.opponent().into()),
        );
        if duel.host_info.time_limit > 0 {
            let time_limit: stoc::Message = stoc::TimeLimit { 
                player,
                left_time: duel.get_player_index(index)?.time_limit
            }.into();
            duel.send(time_limit.clone(), Netplayer::Player(0).into());
            duel.send(time_limit.clone(), Netplayer::Player(1).into());
            duel.get_player_mut_index(index)?.state = Some(ctos::MessageType::TimeConfirm);
        } else {
            duel.get_player_mut_index(index)?.state = Some(ctos::MessageType::Response);
        }
        Some(())
    }

    #[handler(gm::Retry)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_retry(duel: &mut SingleDuel, _message: &gm::Retry) -> Netplayer {
        let netplayer = match duel.last_response {
            Some(player_index) => duel.to_net_player(duel.to_core_player(player_index)),
            None => Netplayer::Unknown,
        };
        netplayer
    }

    #[handler(gm::Hint)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_hint(duel: &mut SingleDuel, message: &gm::Hint) -> SendTarget {
        match message._type {
            Hint::Event | Hint::Message | Hint::SelectMessage | Hint::Effect => {
                SendTarget::Single(duel.to_net_player(message.player))
            }
            Hint::OpponentSelected | Hint::Race | Hint::Attribute | Hint::Code | Hint::Number | Hint::Zone => {
                SendTarget::Except(duel.to_net_player(message.player))
            }
            Hint::Card => SendTarget::All,
        }
    }

    #[handler(gm::Win)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_win(duel: &mut SingleDuel, message: &gm::Win) -> SendTarget {
        // we need send win message before the duel end.
        duel.win_and_end(message.winner.opponent(), message.reason);
        SendTarget::None
    }

    #[handler(gm::SelectBattleCommand)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_battle_command(duel: &mut SingleDuel, message: &gm::SelectBattleCommand) -> Netplayer {
        duel.refresh(CorePlayer::All, Location::MZone | Location::SZone | Location::Hand, -1, Query::empty());
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SelectIdleCommand)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_idle_command(duel: &mut SingleDuel, message: &gm::SelectIdleCommand) -> Netplayer {
        duel.refresh(CorePlayer::All, Location::MZone | Location::SZone | Location::Hand, -1, Query::empty());
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SelectEffectYesNo)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_effect_yes_no(duel: &mut SingleDuel, message: &gm::SelectEffectYesNo) -> Netplayer {
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SelectYesNo)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_yes_no(duel: &mut SingleDuel, message: &gm::SelectYesNo) -> Netplayer {
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SelectOption)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_option(duel: &mut SingleDuel, message: &gm::SelectOption) -> Netplayer {
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SelectCard)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_card(duel: &mut SingleDuel, message: &gm::SelectCard) -> Netplayer {
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SelectChain)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_chain(duel: &mut SingleDuel, message: &gm::SelectChain) -> Netplayer {
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SelectPlace)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_place(duel: &mut SingleDuel, message: &gm::SelectPlace) -> Netplayer {
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SelectPosition)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_position(duel: &mut SingleDuel, message: &gm::SelectPosition) -> Netplayer {
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SelectTribute)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_tribute(duel: &mut SingleDuel, message: &gm::SelectTribute) -> Netplayer {
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SelectCounter)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_counter(duel: &mut SingleDuel, message: &gm::SelectCounter) -> Netplayer {
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SelectSum)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_sum(duel: &mut SingleDuel, message: &gm::SelectSum) -> Netplayer {
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SelectDisableField)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_disable_field(duel: &mut SingleDuel, message: &gm::SelectDisableField) -> Netplayer {
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::SortCard)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_sort_card(duel: &mut SingleDuel, message: &gm::SortCard) -> Netplayer {
        duel.to_net_player(message.player)
    }

    #[handler(gm::SelectUnselectCard)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_unselect_card(duel: &mut SingleDuel, message: &gm::SelectUnselectCard) -> Netplayer {
        duel.to_net_player(message.selecting_player)
    }

    #[handler(gm::ConfirmCards)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_confirm_cards(duel: &mut SingleDuel, message: &gm::ConfirmCards) -> SendTarget {
        let is_deck = message.cards.first().map_or(false, |c| c.location == Location::Deck);
        if is_deck {
            SendTarget::Single(duel.to_net_player(message.player))
        } else {
            SendTarget::All
        }
    }

    #[handler(gm::ShuffleHand)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_shuffle_hand(message: &gm::ShuffleHand) -> (CorePlayer, Location, i8, Query) {
        (message.player, Location::Hand, -1, Query::from_bits_retain(0x781fff))
    }

    #[handler(gm::SwapGraveDeck)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_swap_grave_deck(message: &gm::SwapGraveDeck) -> (CorePlayer, Location) {
        (message.player, Location::Grave)
    }

    #[handler(gm::ShuffleSetCard)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_shuffle_set_card(message: &gm::ShuffleSetCard) -> (CorePlayer, Location, i8, Query) {
        (CorePlayer::All, message.location, -1, Query::from_bits_retain(0x181fff))
    }

    #[handler(gm::ReverseDeck)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_reverse_deck(duel: &mut SingleDuel, _message: &gm::ReverseDeck) {
        duel.deck_reversed = !duel.deck_reversed;
    }

    #[handler(gm::ShuffleExtra)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_shuffle_extra(message: &gm::ShuffleExtra) -> (CorePlayer, Location) {
        (message.player, Location::Extra)
    }

    #[handler(gm::NewTurn)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_new_turn(duel: &mut SingleDuel, message: &gm::NewTurn) -> (CorePlayer, Location) {
        duel.turn_player = message.player;
        let time_limit = duel.host_info.time_limit;
        let time_backed = if duel.configuration.max_add_time_each_turn == 0 { if duel.configuration.add_time_after_operation > 0 { time_limit } else { 0 } } else { duel.configuration.max_add_time_each_turn };
        for duel_player in duel.players.iter_mut().flatten() {
            duel_player.time_limit = time_limit;
            duel_player.time_compensator = 0;
            duel_player.time_backed = time_backed;
        }
        (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
    }

    #[handler(gm::NewPhase)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_new_phase(duel: &mut SingleDuel, message: &gm::NewPhase) -> (CorePlayer, Location) {
        duel.phase = message.phase;
        (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
    }

    #[handler(gm::Move)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_move(message: &gm::Move) -> (CorePlayer, Location, i8) {
        let cc = message.current.controller;
        let cl = message.current.location;
        let cs = message.current.sequence;
        let pc = message.previous.controller;
        let pl = message.previous.location;
        if cl != Location::empty()
            && !cl.intersects(Location::Overlay)
            && (cl != pl || cc != pc)
        {
            (cc, cl, cs as i8)
        } else {
            (CorePlayer::None, Location::empty(), -1)
        }
    }

    #[handler(gm::PositionChange)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_position_change(message: &gm::PositionChange) -> (CorePlayer, Location, i8) {
        if message.previous_position.is_face_down() && !message.current_position.is_face_down() {
            (message.controller, message.location, message.sequence as i8)
        } else {
            (CorePlayer::None, Location::empty(), -1)
        }
    }

    #[handler(gm::Swap)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_swap(duel: &mut SingleDuel, message: &gm::Swap) {
        let p1 = &message.position1;
        let p2 = &message.position2;
        duel.refresh(p1.controller, p1.location, p1.sequence as i8, Query::empty());
        duel.refresh(p2.controller, p2.location, p2.sequence as i8, Query::empty());
    }

    #[handler(gm::Summoned)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_summoned() -> (CorePlayer, Location) {
        (CorePlayer::All, Location::MZone | Location::SZone)
    }

    #[handler(gm::SpecialSummoned)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_special_summoned() -> (CorePlayer, Location) {
        (CorePlayer::All, Location::MZone | Location::SZone)
    }

    #[handler(gm::FlipSummoning)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_flip_summoning(message: &gm::FlipSummoning) -> (CorePlayer, Location, i8) {
        let p = &message.position;
        (p.controller, p.location, p.sequence as i8)
    }

    #[handler(gm::FlipSummoned)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_flip_summoned() -> (CorePlayer, Location) {
        (CorePlayer::All, Location::MZone | Location::SZone)
    }

    #[handler(gm::Chained)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_chained() -> (CorePlayer, Location) {
        (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
    }

    #[handler(gm::ChainSolved)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_chain_solved() -> (CorePlayer, Location) {
        (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
    }

    #[handler(gm::ChainEnd)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_chain_end() -> (CorePlayer, Location) {
        (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
    }

    #[handler(gm::DamageStepStart)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_damage_step_start() -> (CorePlayer, Location) {
        (CorePlayer::All, Location::MZone)
    }

    #[handler(gm::DamageStepEnd)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_damage_step_end() -> (CorePlayer, Location) {
        (CorePlayer::All, Location::MZone)
    }

    #[handler(gm::MissedEffect)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_missed_effect(duel: &mut SingleDuel, message: &gm::MissedEffect) -> Netplayer {
        duel.to_net_player(message.location.controller)
    }

    #[handler(gm::RockPaperScissors)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_rock_paper_scissors(duel: &mut SingleDuel, message: &gm::RockPaperScissors) -> Netplayer {
        duel.to_net_player(message.player)
    }

    #[handler(gm::AnnounceRace)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_announce_race(duel: &mut SingleDuel, message: &gm::AnnounceRace) -> Netplayer {
        duel.to_net_player(message.player)
    }

    #[handler(gm::AnnounceAttribute)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_announce_attribute(duel: &mut SingleDuel, message: &gm::AnnounceAttribute) -> Netplayer {
        duel.to_net_player(message.player)
    }

    #[handler(gm::AnnounceCard)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_announce_card(duel: &mut SingleDuel, message: &gm::AnnounceCard) -> Netplayer {
        duel.to_net_player(message.player)
    }

    #[handler(gm::AnnounceNumber)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_announce_number(duel: &mut SingleDuel, message: &gm::AnnounceNumber) -> Netplayer {
        duel.to_net_player(message.player)
    }

    #[handler(gm::MatchKill)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_match_kill(duel: &mut SingleDuel, message: &gm::MatchKill) {
        duel.match_kill_card_code = message.card_code as i32;
    }
}
