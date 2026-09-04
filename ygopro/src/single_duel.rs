use std::ops::Deref;
use std::ops::DerefMut;

use log::warn;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;

use ygopro_data::constants::*;
use ygopro_data::data::*;
use ygopro_data::message::HostInfo;
use ygopro_data::message::gm::GameMessage;
use ygopro_data::message::{ctos, stoc, gm};
use ygopro_handler::Call;
use ygopro_handler::Bundle;
use ygopro_handler::FromRequest;
use ygopro_handler::MessageKey;

use crate::command::COMMANDS;
use crate::command::CommandHandler;
use crate::configuration::Configuration;
use crate::duel::CorePlayerToSendTarget;
use crate::duel::Duel;
use crate::duel::PlayerIndex;
use crate::duel::Request;
use crate::duel::SendTarget;
use crate::ygopro_handlers::State;

#[repr(C)]
pub struct SingleDuel {
    pub duel: common::Duel,
    pub players: [Option<DuelPlayer>; 2], 
    pub observers: Vec<Option<BaseDuelPlayer>>,
    first_attack_player: Option<PlayerIndex>,
    first_attack_decider: Option<PlayerIndex>,
    pub last_response: Option<PlayerIndex>,
    match_kill_card_code: i32,
    duel_count: u8,
    pub duel_winner: Vec<Option<PlayerIndex>>,
    pub time_elapsed: u16,
    pub last_select_message: Option<gm::Message>,
    // extended by rust ygopro
    response_buffer: BytesMut,
    pub core_request_buffer: BytesMut,
    pub configuration: Configuration,
    timer_task: Option<tokio::task::JoinHandle<()>>,
    pub uninit_players: Vec<Option<BaseDuelPlayer>>,
    // replay recorder
    start_time: u32,
    pub messages: Vec<Complex<stoc::Message>>,
    pub masked_messages: Vec<Complex<stoc::Message>>,
    client_responses: Vec<ctos::Response>,
    // extended by actor models
    pub request_sender: mpsc::UnboundedSender<Request>,
    request_receiver: Option<mpsc::UnboundedReceiver<Request>>,
}

fn log_plugin_statistics(
    enabled_plugins: &hashbrown::HashSet<String>,
    ygopro_handler_counts: &hashbrown::HashMap<&'static str, usize>,
    ygopro_ex_handler_counts: &hashbrown::HashMap<&'static str, usize>,
    ygocore_handler_counts: &hashbrown::HashMap<&'static str, usize>,
    command_counts: &hashbrown::HashMap<&'static str, usize>,
) {
    let mut sorted_plugins = enabled_plugins.iter().collect::<Vec<_>>();
    sorted_plugins.sort();
    log::debug!("enabled plugins and their handlers:");
    for plugin_name in sorted_plugins {
        let ygopro_count = ygopro_handler_counts.get(plugin_name.as_str()).copied().unwrap_or(0);
        let ygopro_ex_count = ygopro_ex_handler_counts.get(plugin_name.as_str()).copied().unwrap_or(0);
        let ygocore_count = ygocore_handler_counts.get(plugin_name.as_str()).copied().unwrap_or(0);
        let command_count = command_counts.get(plugin_name.as_str()).copied().unwrap_or(0);
        let handler_parts = [(ygopro_count, "ygopro handlers"), (ygopro_ex_count, "ygopro ex handlers"), (ygocore_count, "ygocore handlers"), (command_count, "command")]
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, label)| format!("{count} {label}"))
            .collect::<Vec<_>>()
            .join(", ");
        let has_configuration = crate::plugin::CONFIGURATIONS.iter().any(|(name, _)| *name == plugin_name.as_str());
        let configuration_part = if has_configuration { ", configured" } else { "" };
        log::debug!("  {plugin_name}: {handler_parts}{configuration_part}");
    }
}

impl SingleDuel {
    pub(crate) fn new(host_info: HostInfo, configuration: Configuration) -> Self {
        Self {
            duel: Duel::new(host_info, configuration),
            first_attack_player: None,
            last_response: None,
            first_attack_decider: None,
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
            uninit_players: vec![],
            timer_task: None,
        }
    }

    async fn run_processor<Req, Res, Handler>(self, processor: &ygopro_handler::Processor<u8, Req, State<Self>, Res, Handler>, request: Req, states: anymap3::Map<dyn std::any::Any + Send>)
        -> (Self, Req, anymap3::Map<dyn std::any::Any + Send>, Res)
        where Res: Default,
              Req: MessageKey<u8>,
              Handler: Call<Req, State<Self>, Res>
    {
        let state = State { duel: self, states };
        let key = request.message_key();
        let bundle = Bundle::new(request, state, Default::default());
        let Bundle {
            request,
            state: State { duel: returned_duel, states: returned_states },
            response,
            stop_flag: _
        } = processor.process_bundle(bundle, key).await;
        (returned_duel, request, returned_states, response)
    }

    pub fn run(mut self) -> Option<tokio::task::JoinHandle<()>> {
        let receiver = self.request_receiver.take()?;
        let mut stream = UnboundedReceiverStream::new(receiver);

        let handle = tokio::spawn(async move {
            let enabled_groups = &self.configuration.enable_plugins;
            let ygopro_processor = ygopro_handler::Processor::new_with_dual_group(&ygopro_handlers::SINGLE_DUEL_YGOPRO_HANDLERS, &crate::ygopro_handlers::YGOPRO_HANDLERS, &enabled_groups, |handler| handler.module_name, |handler| handler.module_name, |key| *key == 0);
            let ygopro_ex_processor = ygopro_handler::Processor::new_with_dual_group(&ygopro_handlers::SINGLE_DUEL_YGOPRO_HANDLERS_EX, &crate::ygopro_handlers::YGOPRO_HANDLERS_EX, &enabled_groups, |handler| handler.module_name, |handler| handler.module_name, |key| *key == 0);
            let ygocore_processor = ygopro_handler::Processor::new_with_dual_group(&ygocore_handlers::SINGLE_DUEL_YGOCORE_HANDLERS, &crate::ygocore_handlers::YGOCORE_HANDLERS, &enabled_groups, |handler| handler.module_name, |handler| handler.module_name, |key| *key == 0);
            let command_processor: hashbrown::HashMap<&'static str, CommandHandler> = COMMANDS.iter().map(|build| build())
                .filter(|(_, handler)| enabled_groups.contains(handler.module_name)).collect();
            let ygopro_handler_counts = ygopro_processor.handler_statistics(|handler| handler.module_name);
            let ygopro_ex_handler_counts = ygopro_ex_processor.handler_statistics(|handler| handler.module_name);
            let ygocore_handler_counts = ygocore_processor.handler_statistics(|handler| handler.module_name);
            let mut command_counts = hashbrown::HashMap::new();
            for (_, command) in &command_processor {
                *command_counts.entry(command.module_name).or_insert(0) += 1;
            }
            crate::duel::log_plugin_statistics(enabled_groups, &ygopro_handler_counts, &ygopro_ex_handler_counts, &ygocore_handler_counts, &command_counts);
            let mut duel = self;
            let mut states: anymap3::Map<dyn std::any::Any + Send> = anymap3::Map::new();

            while let Some(request) = stream.next().await {
                match request {
                    Request::TimerTick => {
                        if let Some(last_response) = duel.last_response && duel.host_info.time_limit > 0 {
                            duel.time_elapsed = duel.time_elapsed.saturating_add(1);
                            let timed_out = duel.get_player_index(last_response)
                                    .map_or(false, |player| duel.time_elapsed >= player.time_limit);
                            if timed_out {
                                let winner = duel.to_core_player(last_response).opponent();
                                duel.send(gm::Win { winner, reason: WinReason::Timeout }.into(), SendTarget::All);
                                duel.send_request_ex(crate::message::DuelEnd { winner, reason: WinReason::Timeout });
                            }
                        }
                    },
                    Request::Message(request) => {
                        if !duel.get_net(request.extra).map_or(true, |p| p.state.allowed(&request.message)) {
                            warn!("Message type mismatch for player: {:?}, get {:?}", request.extra, ctos::MessageType::from(&request.message));
                            continue;
                        }
                        let (returned_duel, request, returned_states, response) = duel.run_processor(&ygopro_processor, request, states).await;
                        duel = returned_duel;
                        states = returned_states;
                        let position = request.extra;
                        match response {
                            ygopro_handler::extract::Response::Replace(message) => duel.sender.send(message, position.into()),
                            ygopro_handler::extract::Response::ReplaceMultiple(messages) => {
                                let position = position.into();
                                for message in messages {
                                    duel.sender.send(message, position);
                                };
                            },
                            ygopro_handler::extract::Response::Continue => {},
                            ygopro_handler::extract::Response::Swallow => {},
                            ygopro_handler::extract::Response::Terminate => { break; }
                            ygopro_handler::extract::Response::Kick => duel.sender.send(stoc::LeaveGame { pos: position }.into(), position.into())
                        };
                    },
                    Request::MessageEx(request) => {
                        let (returned_duel, request, returned_states, response) = duel.run_processor(&ygopro_ex_processor, request, states).await;
                        duel = returned_duel;
                        states = returned_states;
                        match response {
                            ygopro_handler::extract::Response::Replace(message) => duel.sender.send(message, request.extra),
                            ygopro_handler::extract::Response::ReplaceMultiple(messages) => {
                                for message in messages {
                                    duel.sender.send(message, request.extra);
                                };
                            },
                            ygopro_handler::extract::Response::Continue => request.message.process_continue(&mut duel),
                            ygopro_handler::extract::Response::Swallow => {},
                            ygopro_handler::extract::Response::Terminate => request.message.process_terminate(&mut duel),
                            ygopro_handler::extract::Response::Kick => warn!("unhandled kick response in message ex processing"),
                        };
                    }
                    Request::Evolve => {
                        let messages = crate::ygocore_handlers::evolve(&mut duel);
                        for message in messages {
                            let request = crate::ygocore_handlers::Request { message, extra: Netplayer::Unknown };
                            let (returned_duel, request, returned_states, response) = duel.run_processor(&ygocore_processor, request, states).await;
                            duel = returned_duel;
                            states = returned_states;
                            let (player, locations, sequence, query) = response.refresh;
                            let is_select = request.message.waiting_for().is_some();
                            let transformer = PlayerTransformer(duel.first_attack_player.unwrap_or(PlayerIndex::Player1));
                            if let Some(core_player) = request.message.waiting_for() {
                                duel.set_waiting(core_player);
                                duel.refresh(player, locations, sequence, query, transformer.clone());
                            } else if matches!(&request.message, gm::Message::Retry(_)) {
                                duel.client_responses.pop();
                                if let Some(core_player) = duel.last_response.map(|player| transformer.to_core_player(Netplayer::Player(player.0))) {
                                    duel.set_waiting(core_player);
                                }
                            }
                            duel.send_game_message(request.message, response.target, transformer.clone());
                            if !is_select { duel.refresh(player, locations, sequence, query, transformer.clone()); }
                        }
                    },
                    Request::Command { name, arguments } => {
                        let Some(handler) = command_processor.get(name) else {
                            warn!("no command registered: {name}");
                            continue;
                        };
                        let state = State { duel, states };
                        let bundle = Bundle::new(ygopro_handler::extract::Request { message: arguments, extra: () }, state, Default::default());
                        let bundle = handler.call(bundle).await;
                        let Bundle {
                            request: _,
                            state: State { duel: returned_duel, states: returned_states },
                            response,
                            stop_flag: _
                        } = bundle;
                        duel = returned_duel;
                        states = returned_states;
                        match response {
                            ygopro_handler::extract::Response::Terminate => { break },
                            _ => ()
                        };
                    },
                }
            }
            duel.run_processor(&ygopro_ex_processor, crate::ygopro_handlers::RequestEx { 
                message: crate::message::Terminate.into(), 
                extra: SendTarget::None
            }, states).await;
        });

        Some(handle)
    }

    pub fn create_replay_without_data(&self) -> Option<Replay> {
        let (host_player, client_player) = match self.first_attack_player? {
            PlayerIndex::Player1 => ( self.players[0].as_ref()?, self.players[1].as_ref()? ),
            PlayerIndex::Player2 => ( self.players[1].as_ref()?, self.players[0].as_ref()? ),
            _ => { warn!("first attack player is not a valid player index: {:?}", self.first_attack_player); return None; }
        };
        let seed_sequence = *self.duel.seed();
        let mut duel_options = DuelOptions::empty();
        if self.host_info.no_shuffle_deck {
            duel_options.insert(DuelOptions::PseudoShuffle);
        }
        let host_deck = host_player.deck.clone().into();
        let client_deck = client_player.deck.clone().into();
        let mut replay = Replay {
            header: ReplayHeader {
                id: ReplayVersion::V2 as u32,
                version: *crate::plugin::version_check::PRO_VERSION as u32,
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
                datas: vec![],
            }
        };
        replay.fill_data_size();
        Some(replay)
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
            Netplayer::Undecided(index) => {
                if let Some(Some(uninited)) = &self.uninit_players.get(index as usize) {
                    uninited.stoc_sender.send(message).ok();
                }
            }
            Netplayer::Unknown => warn!("Try to send message to an unknown position.")
        }
    }

    pub(crate) fn _send(&self, message: Complex<stoc::Message>, target: SendTarget) {
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
            SendTarget::Core(coreplayer) => self._send_netplayer(message, self.to_net_player(coreplayer)),
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
        let message = Complex::from_message(stoc::Message::from(message));
        let masked_message = Complex::from_message(stoc::Message::from(masked_message));
        // Select messages and messages already sent by handlers are not recorded.
        if is_waiting_for.is_none() && !matches!(target, SendTarget::None) {
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
                    Netplayer::Observer(_) | Netplayer::Undecided(_) => self._send(masked_message, target),
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
            // todo: CorePlayer::All is not correctly processed.
            SendTarget::Core(coreplayer) => self._send_game_message(message, masked_message, can_player_see_unmasked, SendTarget::Single(self.to_net_player(coreplayer))),
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

    pub fn send_request<Message: Into<ctos::Message>>(&self, message: Message, player: Netplayer) {
        self.request_sender.send(Request::Message( common::Request { message: message.into(), extra: player } )).ok();
    }

    pub fn send_request_ex<Message: Into<crate::message::Message>>(&self, message: Message) {
        self.request_sender.send(Request::MessageEx( common::RequestEx { message: message.into(), extra: SendTarget::All } )).ok();
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
    
    pub fn set_waiting(&mut self, player: CorePlayer) -> Option<()> {
        let transformer = PlayerTransformer(self.first_attack_player.unwrap_or(PlayerIndex::Player1));
        let index = match transformer.to_player_index(player) {
            Some(player) => player,
            None => {
                warn!("try to set waiting to a non-sense player: {:?}", player);
                return None
            }
        };
        self.last_response = Some(index);
        self.sender.send(
            gm::Waiting.into(),
            SendTarget::Single(PlayerIndex(index.0 ^ 1).into()),
        );
        if self.host_info.time_limit > 0 {
            let time_limit: stoc::Message = stoc::TimeLimit {
                player,
                left_time: self.get(index)?.time_limit
            }.into();
            self.sender.send(time_limit.clone(), Netplayer::Player(0).into());
            self.sender.send(time_limit.clone(), Netplayer::Player(1).into());
            self.get_mut(index)?.state = crate::player::AllowMessage::Some(ctos::MessageType::TimeConfirm);
        } else {
            self.get_mut(index)?.state = crate::player::AllowMessage::Some(ctos::MessageType::Response);
        }
        Some(())
    }

}

impl Deref for SingleDuel {
    type Target = Duel;
    fn deref(&self) -> &Self::Target { &self.duel }
}

impl DerefMut for SingleDuel {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.duel }
}

impl AsRef<Duel> for SingleDuel {
    fn as_ref(&self) -> &Duel { &self.duel }
}

impl AsMut<Duel> for SingleDuel {
    fn as_mut(&mut self) -> &mut Duel { &mut self.duel }
}

impl<Message, Extra, Res> FromRequest<ygopro_handler::extract::Request<Message, Extra>, State<SingleDuel>, Res> for &mut SingleDuel
where Message: Send, Extra: Send, Res: Send {
    fn from_request(bundle: &mut Bundle<ygopro_handler::extract::Request<Message, Extra>, State<SingleDuel>, Res>) -> Option<Self> {
        Some(unsafe { &mut *(&mut bundle.state.duel as *mut SingleDuel) })
    }
}

impl<Req, Res> FromRequest<Req, State<SingleDuel>, Res> for &mut Duel
where Req: Send, Res: Send {
    fn from_request(bundle: &mut Bundle<Req, State<SingleDuel>, Res>) -> Option<Self> {
        Some(unsafe { &mut *(&mut bundle.state.duel as *mut SingleDuel as *mut Duel) })
    }
}

unsafe impl ygopro_handler::sync_handler::WithSubState<State<Duel>> for State<SingleDuel> {}
const _: () = {
    assert!(std::mem::offset_of!(SingleDuel, duel) == 0);
    assert!(std::mem::offset_of!(State<Duel>, states) == 0);
    assert!(std::mem::offset_of!(State<SingleDuel>, states) == 0);
    assert!(std::mem::offset_of!(State<Duel>, duel) == std::mem::offset_of!(State<SingleDuel>, duel));
    assert!(std::mem::align_of::<State<Duel>>() == std::mem::align_of::<State<SingleDuel>>());
    assert!(std::mem::align_of::<SingleDuel>() >= std::mem::align_of::<Duel>());
    assert!(std::mem::size_of::<State<Duel>>() <= std::mem::size_of::<State<SingleDuel>>());
    assert!(std::mem::size_of::<Duel>() <= std::mem::size_of::<SingleDuel>());
};

#[derive(Clone)]
pub struct PlayerTransformer(PlayerIndex);
impl CorePlayerToSendTarget for PlayerTransformer {
    fn transform(&self, player: CorePlayer) -> SendTarget {
        let player = if self.0 == PlayerIndex::Player1 { player } else { player.opponent() };
        match player {
            CorePlayer::FirstAttackPlayer => SendTarget::Single(Netplayer::Player(0)),
            CorePlayer::SecondAttackPlayer => SendTarget::Single(Netplayer::Player(1)),
            CorePlayer::All => SendTarget::All,
            CorePlayer::None | CorePlayer::Rule => SendTarget::None,
        }
    }
}

impl PlayerConverter for PlayerTransformer {
    fn to_net_player(&self, core_player: CorePlayer) -> Netplayer {
        let core_player = match self.0 {
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
    
    fn to_core_player(&self, net_player: Netplayer) -> CorePlayer {
        let index = match net_player {
            Netplayer::Player(index) => index,
            _ => return CorePlayer::None
        };
        let core_player = CorePlayer::from(Netplayer::Player(index));
        match self.0 {
            PlayerIndex::Player1 => core_player,
            _ => core_player.opponent(),
        }
    }
}

impl PlayerTransformer {
    pub fn to_player_index(&self, core_player: CorePlayer) -> Option<PlayerIndex> {
        let core_player = match self.0 {
            PlayerIndex::Player1 => core_player,
            _ => core_player.opponent()
        };
        match core_player {
            CorePlayer::FirstAttackPlayer => Some(PlayerIndex::Player1),
            CorePlayer::SecondAttackPlayer => Some(PlayerIndex::Player2),
            _ => None,
        }
    }
}

impl<Req: Send, Res: Send> FromRequest<Req, State<SingleDuel>, Res> for PlayerTransformer {
    fn from_request(bundle: &mut Bundle<Req, State<SingleDuel>, Res>) -> Option<Self> {
        Some(PlayerTransformer(bundle.state.duel.first_attack_player.unwrap_or(PlayerIndex::Player1)))
    }
}


pub mod ygopro_handlers {
    use linkme::distributed_slice;
    use log::warn;
    use ygopro_data::constants::*;
    use ygopro_data::message::ctos;
    use ygopro_data::message::gm;
    use ygopro_data::message::stoc;

    use crate::message as ygopro;
    use crate::duel::PlayerIndex;
    use crate::duel::SendTarget;
    use crate::ygopro_handlers::HandlerTemplate;
    use crate::ygopro_handlers::HandlerExTemplate;

    use super::PlayerTransformer;
    use super::SingleDuel;

    pub type Handler = HandlerTemplate<super::SingleDuel>;
    pub type HandlerEx = HandlerExTemplate<super::SingleDuel>;
    #[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
    pub static NAME: &'static str = module_path!();
    #[distributed_slice]
    pub static SINGLE_DUEL_YGOPRO_HANDLERS: [fn() -> (u8, Handler)];
    #[distributed_slice]
    pub static SINGLE_DUEL_YGOPRO_HANDLERS_EX: [fn() -> (u8, HandlerEx)];

    #[handler(ctos::CreateGame)]
    #[register_to(SINGLE_DUEL_YGOPRO_HANDLERS)]
    fn on_create_game(duel: &mut SingleDuel) {
        duel.max_player_count = 2;
        duel.players = vec![None, None];
    }

    #[handler(ctos::TpResult)]
    #[register_to(SINGLE_DUEL_YGOPRO_HANDLERS)]
    fn on_tp_result(duel: &mut SingleDuel, player: PlayerIndex, tp_result: &ctos::TpResult) {
        duel.first_attack_player = Some(if tp_result.result == CorePlayer::FirstAttackPlayer { player } else { PlayerIndex(player.0 ^ 1) });
    }

    #[handler(ctos::LeaveGame)]
    #[register_to(SINGLE_DUEL_YGOPRO_HANDLERS)]
    fn on_leave_game(duel: &mut SingleDuel, transformer: PlayerTransformer, player: Netplayer) {
        if matches!(player, Netplayer::Player(_)) {
            if duel.stage == DuelStage::Siding {
                duel.sender.send(stoc::DuelStart.into(), SendTarget::AllPlayer);
            }
            if duel.stage != DuelStage::End && !duel.duel.core.ended {
                let Ok(leaving_index) = PlayerIndex::try_from(player) else { return };
                let netplayer: Netplayer = leaving_index.into();
                let loser = transformer.to_core_player(netplayer);
                duel.sender.send(gm::Win { winner: loser.opponent(), reason: WinReason::OpponentLeave }.into(), SendTarget::All);
                duel.queue_request_ex(ygopro::DuelEnd { winner: loser.opponent(), reason: WinReason::OpponentLeave });
            }
        }
    }

    #[handler(ctos::Surrender)]
    #[register_to(SINGLE_DUEL_YGOPRO_HANDLERS)]
    fn on_surrender(duel: &mut SingleDuel, transformer: PlayerTransformer, player: Netplayer) {
        if duel.ended {
            warn!("Surrender requested but duel is already ended.");
            return;
        }
        let winner = transformer.to_core_player(player).opponent();
        duel.sender.send(gm::Win { winner, reason: WinReason::OpponentSurrender }.into(), SendTarget::All);
        duel.queue_request_ex(ygopro::DuelEnd { winner, reason: WinReason::OpponentSurrender });
    }

    #[handler(ctos::Chat)]
    #[register_to(SINGLE_DUEL_YGOPRO_HANDLERS)]
    fn on_chat(duel: &mut SingleDuel, transformer: PlayerTransformer, player: Netplayer, chat: &ctos::Chat) {
        let chat = stoc::Chat {
            player: match player {
                Netplayer::Player(_) => transformer.to_net_player(CorePlayer::from(player)).into(),
                _ => player,
            }.into(),
            msg: chat.msg.clone()
        };
        duel.sender.send(chat.into(), SendTarget::All);
    }

    #[handler(ygopro::FirstShuffle)]
    #[register_to(SINGLE_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_first_shuffle(duel: &mut SingleDuel) {
        if duel.host_info.no_shuffle_deck { return }
        let first_attacker = duel.first_attack_player.unwrap_or(PlayerIndex::Player1);
        let shuffle_order = [first_attacker.0, 1 - first_attacker.0];
        for index in shuffle_order {
            if let Some(deck) = duel.duel.players[index as usize].as_mut().map(|p| &mut p.deck) {
                duel.duel.core.shuffle_deck(&mut deck.main);
            }
        }
    }

    #[handler(ygopro::DuelInit)]
    #[register_to(SINGLE_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_duel_init(duel: &mut SingleDuel) {
        let first_attacker = duel.first_attack_player.unwrap_or(PlayerIndex::Player1);
        let second_attacker = PlayerIndex(first_attacker.0 ^ 1);
        for (player, core_player) in [(first_attacker, CorePlayer::FirstAttackPlayer), (second_attacker, CorePlayer::SecondAttackPlayer)] {
            let Some(deck) = duel.duel.players[player.0 as usize].as_ref() else { return };
            for &code in deck.deck.main.iter().rev() {
                duel.duel.core.new_card(code, core_player, core_player, Location::Deck, 0, Position::FacedownDefense);
            }
            for &code in deck.deck.extra.iter().rev() {
                duel.duel.core.new_card(code, core_player, core_player, Location::Extra, 0, Position::FacedownDefense);
            }
        }
    }

    #[handler(ygopro::DuelStart)]
    #[register_to(SINGLE_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_duel_start(duel: &mut SingleDuel, transformer: PlayerTransformer) {
        let start_lp = duel.host_info.start_lp as i32;
        let duel_rule = duel.host_info.duel_rule;
        let deck1 = duel.query_field_count(CorePlayer::FirstAttackPlayer, Location::Deck) as u16;
        let extra1 = duel.query_field_count(CorePlayer::FirstAttackPlayer, Location::Extra) as u16;
        let deck2 = duel.query_field_count(CorePlayer::SecondAttackPlayer, Location::Deck) as u16;
        let extra2 = duel.query_field_count(CorePlayer::SecondAttackPlayer, Location::Extra) as u16;
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
        let first_attack_netplayer = transformer.to_net_player(CorePlayer::FirstAttackPlayer);
        let second_attack_netplayer = transformer.to_net_player(CorePlayer::SecondAttackPlayer);
        duel.sender.send(start(0).into(), SendTarget::Single(first_attack_netplayer));
        duel.sender.send(start(1).into(), SendTarget::Single(second_attack_netplayer));
        let observer_player_type = match duel.first_attack_player {
            Some(PlayerIndex::Player1) => 0x10,
            Some(PlayerIndex::Player2) => 0x11,
            _ => unreachable!(),
        };
        duel.send(start(observer_player_type).into(), SendTarget::AllObserver);
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
        duel.host_info = create_game.host_info.clone();
        duel.name = create_game.name.clone();
        duel.pass = create_game.pass.clone();
    }

    #[handler(ctos::JoinGame)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_join_game(duel: &mut SingleDuel, player: Netplayer, request: &mut common::Request, join_game: &ctos::JoinGame) -> Result<Vec<stoc::Message>, stoc::Message> {
        if join_game.version != crate::PRO_VERSION {
            return Err(stoc::ErrorMessage { err: ErrorMessage::VersionError(crate::PRO_VERSION) }.into());
        }
        if !duel.pass.is_empty() && join_game.pass != duel.pass {
            return Err(stoc::ErrorMessage { err: ErrorMessage::JoinError(JoinError::WrongPassword) }.into());
        }
        
        // take that player out
        let undecided_index = match player {
            Netplayer::Undecided(index) => index,
            _ => { warn!("Decided player try to send join_game"); return Err(stoc::ErrorMessage { err: ErrorMessage::JoinError(JoinError::HostRefused) }.into()); }
        } as usize;
        let player = match duel.uninit_players.get_mut(undecided_index).map(|c| c.take()) {
            Some(Some(player)) => player,
            _ => { warn!("Cannot find uninit player: {:?}", player); return Err(stoc::ErrorMessage { err: ErrorMessage::JoinError(JoinError::HostRefused) }.into()); }
        };
        while duel.uninit_players.last().is_none() && !duel.uninit_players.is_empty() { duel.uninit_players.pop(); }
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
        let current_netplayer = Netplayer::Observer(SingleDuel::insert_to_last_available_index(&mut duel.observers, duel_player.player) as u8);
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
                    while duel.observers.last().is_none() && !duel.observers.is_empty() { duel.observers.pop(); }
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
                        duel.send(gm::Win { winner: loser.opponent(), reason: WinReason::OpponentLeave }.into(), SendTarget::All);
                        duel.send_request_ex(ygopro::DuelEnd { winner: loser.opponent(), reason: WinReason::OpponentLeave });
                    }
                }
                duel.players[leaving_netplayer as usize] = None;
            }
            Netplayer::Undecided(leaving_player) => {
                if let Some(slot) = duel.uninit_players.get_mut(leaving_player as usize) { slot.take(); }
                while duel.uninit_players.last().is_none() && !duel.uninit_players.is_empty() { duel.uninit_players.pop(); }
            }
            Netplayer::Unknown => {}
        }
        duel.players.iter().all(|player| player.is_none()) && duel.observers.is_empty()
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
        for observer in duel.observers.iter_mut().flatten() {
            observer.state = Some(ctos::MessageType::LeaveGame);
        }

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
        if duel.core.ended {
            warn!("Surrender requested but duel is already ended.");
            return;
        }
        let winner = duel.to_core_player(index).opponent();
        duel.send(gm::Win { winner, reason: WinReason::OpponentSurrender }.into(), SendTarget::All);
        duel.send_request_ex(ygopro::DuelEnd { winner, reason: WinReason::OpponentSurrender });
    }

    #[handler(ctos::TimeConfirm)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_time_confirm(duel: &mut SingleDuel, index: PlayerIndex) {
        if duel.host_info.time_limit == 0 { return; }
        if Some(index) != duel.last_response {
            warn!("TimeConfirm requested by wrong player");
            return;
        }
        let time_elapsed = duel.time_elapsed;
        let Some(duel_player) = duel.get_player_mut_index(index) else {
            warn!("TimeConfirm requested but player slot is empty");
            return;
        };
        duel_player.state = Some(ctos::MessageType::Response);
        duel_player.time_limit = duel_player.time_limit.saturating_sub(time_elapsed);
        duel.time_elapsed = 0;
    }

    #[handler(ctos::Chat)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_chat(duel: &mut SingleDuel, player: Netplayer, chat: &ctos::Chat) {
        // Chat is a protocol which is not related to ygocore, so netplayer should work fine here. But  
        // sadly, all ygopro clients will swap message according to the player is first attack or not.
        // So, we must do a reverse here.
        let chat = stoc::Chat {
            player: match player {
                Netplayer::Player(_) => duel.to_net_player(CorePlayer::from(player)).into(),
                _ => player,
            }.into(),
            msg: chat.msg.clone()
        };
        duel.send(chat.into(), SendTarget::All);
    }

    #[handler(ctos::PlayerInfo)]
    #[register_to(YGOPRO_HANDLERS)]
    fn on_player_info(duel: &mut SingleDuel, player: Netplayer, player_info: &ctos::PlayerInfo) {
        let index = match player {
            Netplayer::Undecided(index) => index,
            _ => { warn!("A decided player {:?} sent player info.", player); return } 
        };
        if let Some(Some(player)) = duel.uninit_players.get_mut(index as usize) {
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
        if kicker != duel.host_player { warn!("HsKick requested by non-host"); return; }
        if duel.stage != DuelStage::Begin { warn!("HsKick requested outside Begin stage"); return; }
        let Netplayer::Player(target) = kick.pos else {
            warn!("HsKick requested to kick non-player");
            return;
        };
        if kicker == kick.pos { warn!("HsKick: cannot kick self"); return; }
        if duel.players[target as usize].is_none() { warn!("HsKick: target slot empty"); return; }
        duel.send(stoc::LeaveGame { pos: kick.pos }.into(), kick.pos.into());
        duel.players[target as usize].take();
        duel.send(stoc::HsPlayerChange {
            status: PlayerChange::new()
                .with_state(PlayerChangeState::Leave)
                .with_player(kick.pos)
        }.into(), SendTarget::Except(kick.pos));
    }

    #[handler(ygopro::ClientJoin)]
    #[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_client_join(duel: &mut SingleDuel, join: &mut ygopro::ClientJoin) {
        if duel.stage > DuelStage::Begin { return }
        if let Some(oneshot) = join.position_sender.take() {
            let player = common::DuelPlayer::new(join.stoc_sender.clone());
            let undecided_index = SingleDuel::insert_to_last_available_index(&mut duel.uninit_players, player);
            oneshot.send(Netplayer::Undecided(undecided_index as u8)).ok();
        } else {
            warn!("ClientJoin try to send the position, but find it already taken.")
        }
    }

    #[handler(ygopro::ClientJoin, priority = 250)]
    #[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_client_join_final(join: &mut ygopro::ClientJoin) {
        if join.position_sender.is_some() {
            let error_message: stoc::Message = stoc::ErrorMessage { err: ErrorMessage::JoinError(JoinError::HostRefused) }.into();
            join.stoc_sender.send(ygopro_data::complex::Complex::from_message(error_message)).ok();
        }
    }

   #[handler(ygopro::FirstShuffle)]
    #[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_first_shuffle(duel: &mut SingleDuel) {
        if duel.host_info.no_shuffle_deck { return }
        let first_attacker = duel.first_attack_player.unwrap_or(PlayerIndex::Player1);
        let shuffle_order = [first_attacker as usize, first_attacker.opponent() as usize];
        for index in shuffle_order {
            if let Some(deck) = duel.players[index].as_mut().map(|p| &mut p.deck) {
                duel.duel.shuffle_deck(&mut deck.main);
            }
        }
    }

    #[handler(ygopro::GenerateReplay)]
    #[register_to(SINGLE_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_generate_replay(duel: &mut SingleDuel) -> Option<stoc::Message> {
        let mut replay = duel.create_replay_without_data()?;
        replay.body.datas = duel.client_responses.clone().into_iter().map(|r| r.response.into()).collect();
        Some(stoc::Replay { replay: Box::new(replay) }.into())
    }

    #[handler(ygopro::DuelEnd)]
    #[register_to(SINGLE_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_duel_end(duel: &mut SingleDuel, transformer: PlayerTransformer, message: &ygopro::DuelEnd) {
        duel.duel_winner.push(transformer.to_player_index(message.winner));
    }

    #[handler(ygopro::JudgeContinueMatch)]
    #[register_to(SINGLE_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_judge_continue_match(duel: &mut SingleDuel) -> &'static str {
        let end_count = if duel.host_info.mode == Mode::Match { 3 } else { 1 };
        let end_win_count = (end_count + 1) / 2;
        let mut player_wins = [0, 0];
        for winner in &duel.duel_winner {
            if let Some(winner) = winner { player_wins[winner.0 as usize] += 1 }
        }
        let should_match_end = duel.duel_winner.len() >= end_count || player_wins[0] >= end_win_count || player_wins[1] >= end_win_count;
        if should_match_end { "terminate" } else { "continue" }
    }

    #[handler(ygopro::JudgeContinueMatch, priority = 250)]
    #[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_judge_continue_match_kill(duel: &mut SingleDuel) -> &'static str {
        if duel.match_kill_card_code > 0 { "terminate" } else { "continue" }
    }

    #[handler(ygopro::JudgeContinueMatch, priority = 251)]
    #[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_judge_continue_somebody_leave(duel: &mut SingleDuel) -> &'static str {
        if duel.players.iter().any(|p| p.is_none()) { "terminate" } else { "continue" }
    }

    #[handler(ygopro::MatchEnd)]
    #[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_match_end(duel: &mut SingleDuel) {
        duel.send(stoc::DuelEnd.into(), SendTarget::All);
    }

    #[handler(ygopro::RecreateDuel)]
    #[register_to(SINGLE_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_recreate_duel(duel: &mut SingleDuel) {
        duel.first_attack_player = None;
        let current_decider = duel.first_attack_decider.unwrap_or(PlayerIndex::Player1);
        let last_winner = duel.duel_winner.last().and_then(|winner| *winner);
        duel.first_attack_decider = Some(last_winner.map(|winner| PlayerIndex(winner.0 ^ 1)).unwrap_or(PlayerIndex(current_decider.0 ^ 1)));
    }
}

pub mod ygocore_handlers {
    use linkme::distributed_slice;

    use ygopro_data::constants::*;
    use ygopro_data::message::gm;
    use ygopro_handler::sync_handler::SyncHandler;

    use crate::duel::SendTarget;
    use crate::single_duel::SingleDuel;
    use crate::ygocore_handlers::Request;
    use crate::ygocore_handlers::Response;
    use crate::ygopro_handlers::State;

    use super::PlayerTransformer;
    
    pub type Handler = SyncHandler<Request, State<SingleDuel>, Response>;

    #[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
    pub static NAME: &'static str = module_path!();
    #[distributed_slice]
    pub static SINGLE_DUEL_YGOCORE_HANDLERS: [fn() -> (u8, Handler)];

    #[handler(gm::Retry)]
    #[register_to(SINGLE_DUEL_YGOCORE_HANDLERS)]
    fn on_retry(duel: &mut SingleDuel, transformer: PlayerTransformer) -> SendTarget {
        duel.last_response.map(|player_index| SendTarget::Core(transformer.to_core_player(Netplayer::Player(player_index.0)))).unwrap_or(SendTarget::None)
    }

    #[handler(gm::Hint)]
    #[register_to(SINGLE_DUEL_YGOCORE_HANDLERS)]
    fn on_hint(transformer: PlayerTransformer, message: &gm::Hint) -> SendTarget {
        match message._type {
            Hint::Event | Hint::Message | Hint::SelectMessage | Hint::Effect => SendTarget::Core(message.player),
            Hint::OpponentSelected | Hint::Race | Hint::Attribute | Hint::Code | Hint::Number | Hint::Zone =>
                SendTarget::Except(transformer.to_net_player(message.player)),
            Hint::Card => SendTarget::All,
        }
    }

    #[handler(gm::Win)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_win(duel: &mut SingleDuel, message: &gm::Win) {
        duel.send_request_ex(ygopro::DuelEnd { winner: message.winner, reason: message.reason });
    }

    #[handler(gm::SelectBattleCommand)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_battle_command(_message: &gm::SelectBattleCommand) -> (CorePlayer, Location, i8, Query) {
        (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand, -1, Query::empty())
    }

    #[handler(gm::SelectIdleCommand)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_select_idle_command(_message: &gm::SelectIdleCommand) -> (CorePlayer, Location, i8, Query) {
        (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand, -1, Query::empty())
    }

    #[handler(gm::ConfirmCards)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_confirm_cards(message: &gm::ConfirmCards) -> SendTarget {
        let is_deck = message.cards.first().map_or(false, |c| c.location == Location::Deck);
        if is_deck {
            SendTarget::Core(message.player)
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

    #[handler(gm::ShuffleExtra)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_shuffle_extra(message: &gm::ShuffleExtra) -> (CorePlayer, Location) {
        (message.player, Location::Extra)
    }

    #[handler(gm::NewTurn)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_new_turn(duel: &mut SingleDuel) -> (CorePlayer, Location) {
        let time_limit = duel.host_info.time_limit;
        for duel_player in duel.players.iter_mut().flatten() {
            duel_player.time_limit = time_limit;
        }
        (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
    }

    #[handler(gm::NewPhase)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_new_phase() -> (CorePlayer, Location) {
        (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
    }

    #[handler(gm::Move)]
    #[register_to(YGOCORE_HANDLERS)]
    fn on_move(message: &gm::Move) -> (CorePlayer, Location, i8) {
        let current_controller = message.current.controller;
        let current_location = message.current.location;
        let current_sequence = message.current.sequence;
        let previous_controller = message.previous.controller;
        let previous_location = message.previous.location;
        if current_location != Location::empty()
            && !current_location.intersects(Location::Overlay)
            && (current_location != previous_location || current_controller != previous_controller)
        {
            (current_controller, current_location, current_sequence as i8)
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
    #[register_to(SINGLE_DUEL_YGOCORE_HANDLERS)]
    fn on_swap(duel: &mut SingleDuel, message: &gm::Swap, transformer: PlayerTransformer) {
        let p1 = &message.position1;
        let p2 = &message.position2;
        duel.refresh(p1.controller, p1.location, p1.sequence as i8, Query::empty(), transformer.clone());
        duel.refresh(p2.controller, p2.location, p2.sequence as i8, Query::empty(), transformer);
    }

    #[handler(gm::MatchKill)]
    #[register_to(SINGLE_DUEL_YGOCORE_HANDLERS)]
    fn on_match_kill(duel: &mut SingleDuel, message: &gm::MatchKill) {
        duel.match_kill_card_code = message.card_code as i32;
    }
}
