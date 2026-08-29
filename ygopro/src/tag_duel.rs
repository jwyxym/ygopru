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
use crate::player::AllowMessage;
use crate::ygopro_handlers::State;

#[derive(Copy, Clone, Eq, PartialEq, Debug, PartialOrd, Ord, Hash)]
pub enum TeamIndex {
    Team1,
    Team2
}

impl TeamIndex {
    pub fn leader(self) -> PlayerIndex {
        match self {
            TeamIndex::Team1 => PlayerIndex::Player1,
            TeamIndex::Team2 => PlayerIndex::Player3,
        }
    }

    pub fn member(self) -> [PlayerIndex; 2] {
        match self {
            TeamIndex::Team1 => [PlayerIndex::Player1, PlayerIndex::Player2],
            TeamIndex::Team2 => [PlayerIndex::Player3, PlayerIndex::Player4],
        }
    }

    pub fn opponent(self) -> Self {
        match self {
            TeamIndex::Team1 => TeamIndex::Team2,
            TeamIndex::Team2 => TeamIndex::Team1,
        }
    }
}

impl PlayerIndex {
    pub fn team(self) -> TeamIndex {
        if self.0 < 2 { TeamIndex::Team1 } else { TeamIndex::Team2 }
    }

    pub fn teammate(self) -> Self {
        PlayerIndex(self.0 ^ 1)
    }

    pub fn opponent(self, first_attack_team: TeamIndex) -> Self {
        let opponent_value = match first_attack_team {
            TeamIndex::Team1 => if self.0 < 2 { 3 - self.0 } else { self.0 - 2 },
            TeamIndex::Team2 => if self.0 < 2 { self.0 + 2 } else { self.0 - 1 },
        };
        PlayerIndex(opponent_value)
    }
}

#[repr(C)]
pub struct TagDuel {
    pub duel: Duel,
    pub first_attack_team: Option<TeamIndex>,
    pub duel_winner: Vec<Option<TeamIndex>>,
    pub current_turn_player: Option<PlayerIndex>,
    pub surrender: [bool; 4],
}

impl TagDuel {
    pub fn new(host_info: HostInfo, configuration: Configuration) -> Self {
        Self {
            duel: Duel::new(host_info, configuration),
            first_attack_team: None,
            duel_winner: vec![],
            current_turn_player: None,
            surrender: [false; 4],
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
            let ygopro_processor = ygopro_handler::Processor::new_with_dual_group(&ygopro_handlers::TAG_DUEL_YGOPRO_HANDLERS, &crate::ygopro_handlers::YGOPRO_HANDLERS, &enabled_groups, |handler| handler.module_name, |handler| handler.module_name, |key| *key == 0);
            let ygopro_ex_processor = ygopro_handler::Processor::new_with_dual_group(&ygopro_handlers::TAG_DUEL_YGOPRO_HANDLERS_EX, &crate::ygopro_handlers::YGOPRO_HANDLERS_EX, &enabled_groups, |handler| handler.module_name, |handler| handler.module_name, |key| *key == 0);
            let ygocore_processor = ygopro_handler::Processor::new_with_dual_group(&ygocore_handlers::TAG_DUEL_YGOCORE_HANDLERS, &crate::ygocore_handlers::YGOCORE_HANDLERS, &enabled_groups, |handler| handler.module_name, |handler| handler.module_name, |key| *key == 0);
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
                            let transformer = duel.player_transformer();
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
        let host_player = self.players[0].as_ref()?;
        let tag_host_player = self.players[1].as_ref()?;
        // the second team leader sits at slot 3, its tag member sits at slot 2
        let client_player = self.players[3].as_ref()?;
        let tag_client_player = self.players[2].as_ref()?;
        let seed_sequence = *self.duel.seed();
        let mut duel_options = DuelOptions::empty();
        if self.host_info.no_shuffle_deck {
            duel_options.insert(DuelOptions::PseudoShuffle);
        }
        duel_options.insert(DuelOptions::TagMode);
        let mut replay = Replay {
            header: ReplayHeader {
                id: ReplayVersion::V2 as u32,
                version: crate::PRO_VERSION as u32,
                flag: ReplayHeaderFlags::Uniform | ReplayHeaderFlags::Compressed | ReplayHeaderFlags::Tag,
                seed: 0,
                data_size: 0,
                start_time: self.start_time,
                props: [93, 0, 0, 128, 0, 0, 0, 0],
                seed_sequence,
                header_version: 1,
                reserved: [0; 3],
            },
            body: ReplayBody {
                // names are written in slot order 0,1,2,3 like the original ygopro
                host_name: host_player.name.clone(),
                tag_host_name: Some(tag_host_player.name.clone()),
                client_name: client_player.name.clone(),
                tag_client_name: Some(tag_client_player.name.clone()),
                start_lp: self.host_info.start_lp,
                start_hand: self.host_info.start_hand as u32,
                draw_count: self.host_info.draw_count as u32,
                duel_options,
                duel_rule: self.host_info.duel_rule as u16,
                host_deck: host_player.deck.clone().into(),
                tag_host_deck: Some(tag_host_player.deck.clone().into()),
                client_deck: client_player.deck.clone().into(),
                tag_client_deck: Some(tag_client_player.deck.clone().into()),
                datas: vec![],
            }
        };
        replay.fill_data_size();
        Some(replay)
    }

    pub fn set_waiting(&mut self, player: CorePlayer) -> Option<()> {
        let transformer = self.player_transformer();
        let index = transformer.to_player_index(player)?;
        self.last_response = Some(index);
        for player_index in 0..4u8 {
            let netplayer = Netplayer::Player(player_index);
            if netplayer != index.into() {
                self.sender.send(gm::Waiting.into(), SendTarget::Single(netplayer));
            }
        }
        if self.host_info.time_limit > 0 {
            let time_limit: stoc::Message = stoc::TimeLimit {
                player,
                left_time: self.get(index)?.time_limit
            }.into();
            for player_index in 0..4u8 {
                self.sender.send(time_limit.clone(), Netplayer::Player(player_index).into());
            }
            self.get_mut(index)?.state = AllowMessage::Some(ctos::MessageType::TimeConfirm);
        } else {
            self.get_mut(index)?.state = AllowMessage::Some(ctos::MessageType::Response);
        }
        Some(())
    }

    pub fn player_transformer(&self) -> PlayerTransformer {
        PlayerTransformer::new(self.first_attack_team, self.current_turn_player)
    }

    pub fn rotate_turn_player(&mut self, player: CorePlayer) {
        let first_attack_team = self.first_attack_team.unwrap_or(TeamIndex::Team1);
        let operator = if self.current_turn_player == Some(first_attack_team.leader())
            && player == CorePlayer::FirstAttackPlayer {
            first_attack_team.leader()
        } else {
            match self.current_turn_player {
                Some(current) => current.teammate().opponent(first_attack_team),
                None => first_attack_team.leader(),
            }
        };
        self.current_turn_player = Some(operator);
    }
}

impl Deref for TagDuel {
    type Target = Duel;
    fn deref(&self) -> &Self::Target { &self.duel }
}

impl DerefMut for TagDuel {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.duel }
}

impl AsRef<Duel> for TagDuel {
    fn as_ref(&self) -> &Duel { &self.duel }
}

impl AsMut<Duel> for TagDuel {
    fn as_mut(&mut self) -> &mut Duel { &mut self.duel }
}

impl<Message, Extra, Res> FromRequest<ygopro_handler::extract::Request<Message, Extra>, State<TagDuel>, Res> for &mut TagDuel
where Message: Send, Extra: Send, Res: Send {
    fn from_request(bundle: &mut Bundle<ygopro_handler::extract::Request<Message, Extra>, State<TagDuel>, Res>) -> Option<Self> {
        Some(unsafe { &mut *(&mut bundle.state.duel as *mut TagDuel) })
    }
}

impl<Req, Res> FromRequest<Req, State<TagDuel>, Res> for &mut Duel
where Req: Send, Res: Send {
    fn from_request(bundle: &mut Bundle<Req, State<TagDuel>, Res>) -> Option<Self> {
        Some(unsafe { &mut *(&mut bundle.state.duel as *mut TagDuel as *mut Duel) })
    }
}

unsafe impl ygopro_handler::sync_handler::WithSubState<State<Duel>> for State<TagDuel> {}
const _: () = {
    assert!(std::mem::offset_of!(TagDuel, duel) == 0);
    assert!(std::mem::offset_of!(State<Duel>, states) == 0);
    assert!(std::mem::offset_of!(State<TagDuel>, states) == 0);
    assert!(std::mem::offset_of!(State<Duel>, duel) == std::mem::offset_of!(State<TagDuel>, duel));
    assert!(std::mem::align_of::<State<Duel>>() == std::mem::align_of::<State<TagDuel>>());
    assert!(std::mem::align_of::<TagDuel>() >= std::mem::align_of::<Duel>());
    assert!(std::mem::size_of::<State<Duel>>() <= std::mem::size_of::<State<TagDuel>>());
    assert!(std::mem::size_of::<Duel>() <= std::mem::size_of::<TagDuel>());
};

#[derive(Clone)]
pub struct PlayerTransformer {
    first_attack_team: TeamIndex,
    current_turn_player: Option<PlayerIndex>,
}

impl PlayerTransformer {
    pub fn new(first_attack_team: Option<TeamIndex>, current_turn_player: Option<PlayerIndex>) -> Self {
        Self { first_attack_team: first_attack_team.unwrap_or(TeamIndex::Team1), current_turn_player }
    }

    pub fn to_player_index(&self, core_player: CorePlayer) -> Option<PlayerIndex> {
        match core_player {
            CorePlayer::FirstAttackPlayer | CorePlayer::SecondAttackPlayer => {
                let current = self.current_turn_player?;
                let current_team_is_first = match self.first_attack_team {
                    TeamIndex::Team1 => current.team() == TeamIndex::Team1,
                    TeamIndex::Team2 => current.team() == TeamIndex::Team2,
                };
                let target_is_first = core_player == CorePlayer::FirstAttackPlayer;
                if current_team_is_first == target_is_first {
                    Some(current)
                } else {
                    Some(current.opponent(self.first_attack_team))
                }
            }
            _ => None,
        }
    }

    pub fn team_to_core_player(&self, team: TeamIndex) -> CorePlayer {
        match self.first_attack_team {
            TeamIndex::Team1 => if team == TeamIndex::Team1 { CorePlayer::FirstAttackPlayer } else { CorePlayer::SecondAttackPlayer },
            TeamIndex::Team2 => if team == TeamIndex::Team2 { CorePlayer::FirstAttackPlayer } else { CorePlayer::SecondAttackPlayer },
        }
    }

    pub fn to_core_player(&self, net_player: Netplayer) -> CorePlayer {
        let Netplayer::Player(index) = net_player else { return CorePlayer::None };
        let player = PlayerIndex(index);
        let Some(current) = self.current_turn_player else { return self.team_to_core_player(player.team()) };
        let current_opponent = current.opponent(self.first_attack_team);
        if player == current || player == current_opponent {
            self.team_to_core_player(player.team())
        } else {
            CorePlayer::Rule
        }
    }
}

impl CorePlayerToSendTarget for PlayerTransformer {
    fn transform(&self, player: CorePlayer) -> SendTarget {
        match self.to_player_index(player) {
            Some(index) => SendTarget::Single(index.into()),
            None => match player {
                CorePlayer::All => SendTarget::All,
                _ => SendTarget::None,
            },
        }
    }
}

impl PlayerConverter for PlayerTransformer {
    fn to_net_player(&self, core_player: CorePlayer) -> Netplayer {
        self.to_player_index(core_player).map(|index| index.into()).unwrap_or(Netplayer::Unknown)
    }

    fn to_core_player(&self, net_player: Netplayer) -> CorePlayer {
        PlayerTransformer::to_core_player(self, net_player)
    }
}

impl<Req: Send, Res: Send> FromRequest<Req, State<TagDuel>, Res> for PlayerTransformer {
    fn from_request(bundle: &mut Bundle<Req, State<TagDuel>, Res>) -> Option<Self> {
        Some(bundle.state.duel.player_transformer())
    }
}

pub mod ygopro_handlers {
    use linkme::distributed_slice;
    use log::warn;
    use ygopro_data::constants::*;
    use ygopro_data::message::ctos;
    use ygopro_data::message::gm;
    use ygopro_data::message::stoc;
    use ygopro_handler::StopFlag;

    use crate::message as ygopro;
    use crate::duel::PlayerIndex;
    use crate::duel::SendTarget;
    use crate::player::AllowMessage;
    use crate::ygopro_handlers::HandlerExTemplate;
    use crate::ygopro_handlers::HandlerTemplate;

    use super::PlayerTransformer;
    use super::TagDuel;
    use super::TeamIndex;

    pub type Handler = HandlerTemplate<TagDuel>;
    pub type HandlerEx = HandlerExTemplate<TagDuel>;

    #[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
    pub static NAME: &'static str = module_path!();
    #[distributed_slice]
    pub static TAG_DUEL_YGOPRO_HANDLERS: [fn() -> (u8, Handler)];
    #[distributed_slice]
    pub static TAG_DUEL_YGOPRO_HANDLERS_EX: [fn() -> (u8, HandlerEx)];

    #[handler(ctos::CreateGame)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS)]
    fn on_create_game(duel: &mut TagDuel) {
        duel.max_player_count = 4;
        duel.players = vec![None, None, None, None];
    }

    #[handler(ctos::TpResult)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS)]
    fn on_tp_result(duel: &mut TagDuel, player: PlayerIndex, tp_result: &ctos::TpResult) {
        let team = player.team();
        let first_attack_team = if tp_result.result == CorePlayer::FirstAttackPlayer { team } else { team.opponent() };
        duel.first_attack_team = Some(first_attack_team);
        duel.current_turn_player = Some(first_attack_team.leader());
    }

    #[handler(ctos::LeaveGame)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS)]
    fn on_leave_game(duel: &mut TagDuel, player: Netplayer) {
        if duel.stage != DuelStage::Begin && duel.stage != DuelStage::End && !duel.duel.core.ended {
            if let Ok(leaving_index) = PlayerIndex::try_from(player) {
                let winner = duel.player_transformer().team_to_core_player(leaving_index.team().opponent());
                duel.sender.send(gm::Win { winner, reason: WinReason::OpponentLeave }.into(), SendTarget::All);
                duel.queue_request_ex(ygopro::DuelEnd { winner, reason: WinReason::OpponentLeave });
            }
        }
    }

    #[handler(ctos::Surrender)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS)]
    fn on_surrender(duel: &mut TagDuel, player: PlayerIndex) {
        if duel.ended {
            warn!("Surrender requested but duel is already ended.");
            return;
        }
        let team = player.team();
        if duel.surrender[player.0 as usize] { return; }
        if !duel.surrender[player.teammate().0 as usize] {
            duel.surrender[player.0 as usize] = true;
            duel.sender.send(stoc::TeammateSurrender.into(), SendTarget::Single(player.into()));
            duel.sender.send(stoc::TeammateSurrender.into(), SendTarget::Single(player.teammate().into()));
            return;
        }
        let winner = duel.player_transformer().team_to_core_player(team.opponent());
        duel.sender.send(gm::Win { winner, reason: WinReason::OpponentSurrender }.into(), SendTarget::All);
        duel.queue_request_ex(ygopro::DuelEnd { winner, reason: WinReason::OpponentSurrender });
    }

    #[handler(ctos::Chat)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS)]
    fn on_chat(duel: &mut TagDuel, player: Netplayer, chat: &ctos::Chat) {
        let chat = stoc::Chat { player: player.into(), msg: chat.msg.clone() };
        duel.sender.send(chat.into(), SendTarget::All);
    }

    #[before(ctos::HsStart)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS)]
    fn on_hs_start(duel: &mut TagDuel, stop: &mut StopFlag, player: Netplayer) {
        stop.0 = true;
        if player != duel.host_player {
            warn!("HsStart requested by non-host");
            return;
        }
        if !duel.players.iter().all(|p| p.as_ref().map(|p| p.ready) == Some(true)) {
            warn!("HsStart: not all players ready");
            return;
        }
        duel.stage = DuelStage::Finger;
        duel.sender.send(stoc::DuelStart.into(), SendTarget::All);
        for (_, observer) in duel.observers.iter_mut() {
            observer.state = AllowMessage::None;
        }
        let team1_deck = duel.get(TeamIndex::Team1.leader()).map(|p| (p.deck.main.len(), p.deck.side.len(), p.deck.extra.len()));
        let team2_deck = duel.get(TeamIndex::Team2.leader()).map(|p| (p.deck.main.len(), p.deck.side.len(), p.deck.extra.len()));
        let (Some((team1_main, team1_side, team1_extra)), Some((team2_main, team2_side, team2_extra))) = (team1_deck, team2_deck) else { return };
        let team1_count = stoc::DeckCount {
            mainc_s: team1_main as u16, sidec_s: team1_side as u16, extrac_s: team1_extra as u16,
            mainc_o: team2_main as u16, sidec_o: team2_side as u16, extrac_o: team2_extra as u16,
        };
        let team2_count = stoc::DeckCount {
            mainc_s: team2_main as u16, sidec_s: team2_side as u16, extrac_s: team2_extra as u16,
            mainc_o: team1_main as u16, sidec_o: team1_side as u16, extrac_o: team1_extra as u16,
        };
        for player in TeamIndex::Team1.member() {
            duel.sender.send(team1_count.clone().into(), SendTarget::Single(player.into()));
        }
        for player in TeamIndex::Team2.member() {
            duel.sender.send(team2_count.clone().into(), SendTarget::Single(player.into()));
        }
        let leader1 = TeamIndex::Team1.leader();
        let leader2 = TeamIndex::Team2.leader();
        for leader in [leader1, leader2] {
            duel.sender.send(stoc::SelectHand.into(), SendTarget::Single(leader.into()));
        }
        let [l1, l2] = duel.get_many_mut([leader1, leader2]);
        if let Some(player) = l1.as_mut() { player.hand = None; player.state = AllowMessage::Some(ctos::MessageType::HandResult); }
        if let Some(player) = l2.as_mut() { player.hand = None; player.state = AllowMessage::Some(ctos::MessageType::HandResult); }
    }

    #[before(ctos::HandResult)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS)]
    fn on_hand_result(duel: &mut TagDuel, stop: &mut StopFlag, player: PlayerIndex, hand_result: &ctos::HandResult) {
        stop.0 = true;
        let leader1 = TeamIndex::Team1.leader();
        let leader2 = TeamIndex::Team2.leader();
        if player != leader1 && player != leader2 { return; }
        if let Some(duel_player) = duel.get_mut(player) {
            duel_player.hand = Some(hand_result.res);
        }
        let hand1 = match duel.get(leader1).and_then(|p| p.hand) { Some(hand) => hand, None => return };
        let hand2 = match duel.get(leader2).and_then(|p| p.hand) { Some(hand) => hand, None => return };
        let observer_message = stoc::HandResult { hand1, hand2 };
        let result = observer_message.judge();
        match result {
            HandResult::Draw => {
                let [l1, l2] = duel.get_many_mut([leader1, leader2]);
                if let Some(player) = l1.as_mut() { player.hand = None; player.state = AllowMessage::Some(ctos::MessageType::HandResult); }
                if let Some(player) = l2.as_mut() { player.hand = None; player.state = AllowMessage::Some(ctos::MessageType::HandResult); }
                duel.sender.send(stoc::SelectHand.into(), SendTarget::Single(leader1.into()));
                duel.sender.send(stoc::SelectHand.into(), SendTarget::Single(leader2.into()));
            },
            HandResult::Win => {
                let [l1, l2] = duel.get_many_mut([leader1, leader2]);
                if let Some(player) = l1.as_mut() { player.state = AllowMessage::Some(ctos::MessageType::TpResult); }
                if let Some(player) = l2.as_mut() { player.state = AllowMessage::None; }
                duel.first_attack_decider = Some(leader1);
                duel.sender.send(stoc::SelectTp.into(), SendTarget::Single(leader1.into()));
                duel.stage = DuelStage::Firstgo;
            },
            HandResult::Lose => {
                let [l1, l2] = duel.get_many_mut([leader1, leader2]);
                if let Some(player) = l1.as_mut() { player.state = AllowMessage::None; }
                if let Some(player) = l2.as_mut() { player.state = AllowMessage::Some(ctos::MessageType::TpResult); }
                duel.first_attack_decider = Some(leader2);
                duel.sender.send(stoc::SelectTp.into(), SendTarget::Single(leader2.into()));
                duel.stage = DuelStage::Firstgo;
            }
        }
        duel.sender.send(observer_message.swap_clone().into(), SendTarget::Single(Netplayer::Player(2)));
        duel.sender.send(observer_message.swap_clone().into(), SendTarget::Single(Netplayer::Player(3)));
        duel.sender.send(observer_message.clone().into(), SendTarget::Single(Netplayer::Player(0)));
        duel.sender.send(observer_message.into(), SendTarget::Single(Netplayer::Player(1)));
        duel.sender.send(stoc::HandResult { hand1, hand2 }.into(), SendTarget::AllObserver);
    }

    #[handler(ygopro::FirstShuffle)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_first_shuffle(duel: &mut TagDuel) {
        if duel.host_info.no_shuffle_deck { return }
        let first_attack_team = duel.first_attack_team.unwrap_or(TeamIndex::Team1);
        let second_attack_team = first_attack_team.opponent();
        let order = [
            first_attack_team.leader(),
            first_attack_team.leader().teammate(),
            second_attack_team.leader().teammate(),
            second_attack_team.leader(),
        ];
        for index in order {
            if let Some(deck) = duel.duel.players[index.0 as usize].as_mut().map(|p| &mut p.deck) {
                duel.duel.core.shuffle_deck(&mut deck.main);
            }
        }
    }

    #[handler(ygopro::DuelInit)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_duel_init(duel: &mut TagDuel) {
        let first_attack_team = duel.first_attack_team.unwrap_or(TeamIndex::Team1);
        for (core_player, team) in [(CorePlayer::FirstAttackPlayer, first_attack_team), (CorePlayer::SecondAttackPlayer, first_attack_team.opponent())] {
            let leader = team.leader();
            let member = leader.teammate();
            let Some(leader_deck) = duel.duel.players[leader.0 as usize].as_ref() else { return };
            let Some(member_deck) = duel.duel.players[member.0 as usize].as_ref() else { return };
            for &code in leader_deck.deck.main.iter().rev() {
                duel.duel.core.new_card(code, core_player, core_player, Location::Deck, 0, Position::FacedownDefense);
            }
            for &code in leader_deck.deck.extra.iter().rev() {
                duel.duel.core.new_card(code, core_player, core_player, Location::Extra, 0, Position::FacedownDefense);
            }
            for &code in member_deck.deck.main.iter().rev() {
                duel.duel.core.new_tag_card(code, core_player, Location::Deck);
            }
            for &code in member_deck.deck.extra.iter().rev() {
                duel.duel.core.new_tag_card(code, core_player, Location::Extra);
            }
        }
    }

    #[handler(ygopro::DuelStart)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_duel_start(duel: &mut TagDuel, transformer: PlayerTransformer) {
        let first_attack_team = duel.first_attack_team.unwrap_or(TeamIndex::Team1);
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
        let (first_attack_members, second_attack_members) = if first_attack_team == TeamIndex::Team1 {
            (TeamIndex::Team1.member(), TeamIndex::Team2.member())
        } else {
            (TeamIndex::Team2.member(), TeamIndex::Team1.member())
        };
        for player in first_attack_members { duel.sender.send(start(0).into(), SendTarget::Single(player.into())); }
        for player in second_attack_members { duel.sender.send(start(1).into(), SendTarget::Single(player.into())); }
        let observer_player_type = if first_attack_team == TeamIndex::Team1 { 0x10 } else { 0x11 };
        duel.sender.send(start(observer_player_type).into(), SendTarget::AllObserver);
        duel.refresh(CorePlayer::All, Location::Extra, -1, Query::empty(), transformer);
    }

    #[handler(ygopro::GenerateReplay)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_generate_replay(duel: &mut TagDuel) -> Option<stoc::Message> {
        let mut replay = duel.create_replay_without_data()?;
        replay.body.datas = duel.client_responses.clone().into_iter().map(|r| r.response.into()).collect();
        Some(stoc::Replay { replay: Box::new(replay) }.into())
    }

    #[handler(ygopro::DuelEnd)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_duel_end(duel: &mut TagDuel, message: &ygopro::DuelEnd) {
        let winner_team = match message.winner {
            CorePlayer::FirstAttackPlayer => duel.first_attack_team,
            CorePlayer::SecondAttackPlayer => duel.first_attack_team.map(|team| team.opponent()),
            _ => None,
        };
        duel.duel_winner.push(winner_team);
    }

    #[handler(ygopro::JudgeContinueMatch)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_judge_continue_match(duel: &mut TagDuel) -> &'static str {
        let end_count = if duel.host_info.mode == Mode::Match { 3 } else { 1 };
        let end_win_count = (end_count + 1) / 2;
        let mut team_wins = [0, 0];
        for winner in &duel.duel_winner {
            if let Some(winner) = winner { team_wins[*winner as usize] += 1 }
        }
        let should_match_end = duel.duel_winner.len() >= end_count || team_wins[0] >= end_win_count || team_wins[1] >= end_win_count;
        if should_match_end { "terminate" } else { "continue" }
    }

    #[handler(ygopro::RecreateDuel)]
    #[register_to(TAG_DUEL_YGOPRO_HANDLERS_EX as HandlerEx)]
    fn on_recreate_duel(duel: &mut TagDuel) {
        let current_decider = duel.first_attack_decider.unwrap_or(TeamIndex::Team1.leader());
        let last_winner = duel.duel_winner.last().and_then(|winner| *winner);
        duel.first_attack_team = None;
        duel.current_turn_player = None;
        duel.surrender = [false; 4];
        duel.first_attack_decider = Some(last_winner.map(|winner| winner.opponent().leader()).unwrap_or(PlayerIndex(current_decider.0 ^ 1)));
    }
}

pub mod ygocore_handlers {
    use linkme::distributed_slice;
    use ygopro_data::constants::*;
    use ygopro_data::message::gm;
    use ygopro_handler::sync_handler::SyncHandler;

    use crate::duel::SendTarget;
    use crate::ygocore_handlers::Request;
    use crate::ygocore_handlers::Response;
    use crate::ygopro_handlers::State;

    use super::PlayerTransformer;
    use super::TagDuel;

    pub type Handler = SyncHandler<Request, State<TagDuel>, Response>;

    #[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
    pub static NAME: &'static str = module_path!();
    #[distributed_slice]
    pub static TAG_DUEL_YGOCORE_HANDLERS: [fn() -> (u8, Handler)];

    #[handler(gm::Retry)]
    #[register_to(TAG_DUEL_YGOCORE_HANDLERS)]
    fn on_retry(duel: &mut TagDuel, transformer: PlayerTransformer) -> SendTarget {
        duel.last_response.map(|player_index| SendTarget::Core(transformer.to_core_player(Netplayer::Player(player_index.0)))).unwrap_or(SendTarget::None)
    }

    #[handler(gm::NewTurn)]
    #[register_to(TAG_DUEL_YGOCORE_HANDLERS)]
    fn on_new_turn(duel: &mut TagDuel, message: &gm::NewTurn) -> (CorePlayer, Location) {
        duel.rotate_turn_player(message.player);
        (CorePlayer::All, Location::MZone | Location::SZone | Location::Hand)
    }

    #[handler(gm::Hint)]
    #[register_to(TAG_DUEL_YGOCORE_HANDLERS)]
    fn on_hint(transformer: PlayerTransformer, message: &gm::Hint) -> SendTarget {
        match message._type {
            Hint::Event | Hint::Message | Hint::SelectMessage | Hint::Effect => SendTarget::Core(message.player),
            Hint::OpponentSelected | Hint::Race | Hint::Attribute | Hint::Code | Hint::Number | Hint::Zone =>
                SendTarget::Except(transformer.to_net_player(message.player)),
            Hint::Card => SendTarget::All,
        }
    }

    #[handler(gm::Swap)]
    #[register_to(TAG_DUEL_YGOCORE_HANDLERS)]
    fn on_swap(duel: &mut TagDuel, message: &gm::Swap, transformer: PlayerTransformer) {
        let p1 = &message.position1;
        let p2 = &message.position2;
        duel.refresh(p1.controller, p1.location, p1.sequence as i8, Query::empty(), transformer.clone());
        duel.refresh(p2.controller, p2.location, p2.sequence as i8, Query::empty(), transformer);
    }

    #[handler(gm::TagSwap)]
    #[register_to(TAG_DUEL_YGOCORE_HANDLERS)]
    fn on_tag_swap() -> (CorePlayer, Location) {
        (CorePlayer::All, Location::Extra | Location::MZone | Location::SZone | Location::Hand)
    }
}
