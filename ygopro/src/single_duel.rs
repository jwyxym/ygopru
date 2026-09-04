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
    pub duel: Duel,
    pub first_attack_player: Option<PlayerIndex>,
    pub duel_winner: Vec<Option<PlayerIndex>>
}

impl SingleDuel {
    pub(crate) fn new(host_info: HostInfo, configuration: Configuration) -> Self {
        Self {
            duel: Duel::new(host_info, configuration),
            first_attack_player: None,
            duel_winner: vec![]
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
                        let bundle = Bundle::new(arguments.unwrap_or_else(|| Box::new(())), state, Default::default());
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
            if duel.stage > DuelStage::Begin && duel.stage != DuelStage::End && !duel.duel.core.ended {
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
        duel.sender.send(start(observer_player_type).into(), SendTarget::AllObserver);
        duel.refresh(CorePlayer::All, Location::Extra, -1, Query::empty(), transformer);
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
