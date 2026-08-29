use std::io::Cursor;
use std::ops::{Deref, DerefMut};

use binrw::BinWrite;
use linkme::distributed_slice;
use log::warn;
use ygopro_data::constants::*;
use ygopro_data::data::DuelOptions;
use ygopro_data::message::{ctos, stoc, gm};
use ygopro_handler::extract::{ContainsMap, ContainsMapMut};
use ygopro_handler::*;
use ygopro_handler::sync_handler::SyncHandler;

use crate::message as ygopro;
use crate::duel::*;
use crate::managers::*;
use crate::player::AllowMessage;

#[distributed_slice(crate::plugin::DEFAULT_ENABLED_PLUGINS)]
pub static NAME: &'static str = module_path!();

pub type Request = ygopro_handler::extract::Request<ctos::Message, Netplayer>; 
pub type RequestEx = ygopro_handler::extract::Request<crate::message::Message, SendTarget>;
pub type Response = ygopro_handler::extract::Response<stoc::Message>;
pub type HandlerTemplate<Duel> = SyncHandler<Request, State<Duel>, Response>;
pub type HandlerExTemplate<Duel> = SyncHandler<RequestEx, State<Duel>, Response>;
pub type Handler = HandlerTemplate<Duel>;
pub type HandlerEx = HandlerExTemplate<Duel>;

#[repr(C)]
pub struct State<Duel: 'static> {
    pub states: anymap3::Map<dyn std::any::Any + Send>,
    pub duel: Duel,
}

impl<Duel> Deref for State<Duel> {
    type Target = Duel;

    fn deref(&self) -> &Self::Target {
        &self.duel
    }
}

impl<Duel> DerefMut for State<Duel> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.duel
    }
}

impl<Duel: 'static> ContainsMapMut for State<Duel> {
    fn get_map(&mut self) -> &mut anymap3::Map<dyn std::any::Any + Send> {
        &mut self.states
    }
}

impl<TDuel: 'static> ContainsMap for State<TDuel> where TDuel: Deref<Target = Duel> {
    fn get_map(&self) -> &anymap3::Map<dyn anymap3::CloneAny + Send> {
        &self.duel.configuration.configurations
    }
}

impl ContainsMap for State<Duel> {
    fn get_map(&self) -> &anymap3::Map<dyn anymap3::CloneAny + Send> {
        &self.duel.configuration.configurations
    }
}

impl<Req, Res> FromRequest<Req, State<Duel>, Res> for &mut Duel where Req: Send, Res: Send {
    fn from_request(bundle: &mut Bundle<Req, State<Duel>, Res>) -> Option<Self> {
        Some(unsafe { &mut *(&mut bundle.state.duel as *mut Duel) })
    }
}

impl<Res> FromRequest<Request, State<Duel>, Res> for &mut Request where Res: Send {
    fn from_request(bundle: &mut Bundle<Request, State<Duel>, Res>) -> Option<Self> {
        Some(unsafe { &mut *(&mut bundle.request as *mut Request) })
    }
}

impl<Res> FromRequest<Request, State<Duel>, Res> for PlayerIndex where Res: Send {
    fn from_request(bundle: &mut Bundle<Request, State<Duel>, Res>) -> Option<Self> {
        let s = Self::try_from(bundle.request.extra).ok()?;
        if s.0 as usize >= bundle.state.max_player_count { return None }
        Some(s)
    }
}

impl<Res, TDuel> FromRequest<Request, State<TDuel>, Res> for PlayerIndex where Res: Send, TDuel: Deref<Target = Duel>, TDuel: Send {
    fn from_request(bundle: &mut Bundle<Request, State<TDuel>, Res>) -> Option<Self> {
        let s = Self::try_from(bundle.request.extra).ok()?;
        if s.0 as usize >= bundle.state.max_player_count { return None }
        Some(s)
    }
}

#[distributed_slice]
pub static YGOPRO_HANDLERS: [fn() -> (u8, Handler)];
#[distributed_slice]
pub static YGOPRO_HANDLERS_EX: [fn() -> (u8, HandlerEx)];

#[handler(ctos::Response)]
#[register_to(YGOPRO_HANDLERS)]
fn on_response(duel: &mut Duel, player: PlayerIndex, response: &ctos::Response) {
    if duel.ended { return; }
    duel.client_responses.push(response.clone());
    {
        duel.response_buffer.fill(0);
        let mut cursor = Cursor::new(&mut duel.response_buffer[..]);
        response.write_le(&mut cursor).ok();
    }
    duel.set_responseb(&duel.response_buffer);
    if let Some(duel_player) = duel.get_mut(player) {
        duel_player.state = AllowMessage::None;
    }
    duel.request_sender.send(crate::duel::Request::Evolve).ok();
}

#[handler(ctos::HandResult)]
#[register_to(YGOPRO_HANDLERS)]
fn on_hand_result(duel: &mut Duel, player: PlayerIndex, hand_result: &ctos::HandResult) {
    if let Some(duel_player) = duel.get_mut(player) {
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
                player1.state = AllowMessage::Some(ctos::MessageType::HandResult);
                player2.state = AllowMessage::Some(ctos::MessageType::HandResult);
                duel.sender.send(stoc::SelectHand.into(), SendTarget::AllPlayer);
                (observer_message, None)
            },
            HandResult::Win => {
                player1.state = AllowMessage::Some(ctos::MessageType::TpResult);
                player2.state = AllowMessage::None;
                (observer_message, Some(PlayerIndex::Player1))
            },
            HandResult::Lose => {
                player1.state = AllowMessage::None;
                player2.state = AllowMessage::Some(ctos::MessageType::TpResult);
                (observer_message, Some(PlayerIndex::Player2))
            }
        }
    };
    
    duel.sender.send(message.swap_clone().into(), SendTarget::Single(Netplayer::Player(1)));
    duel.sender.send(message.into(), SendTarget::Except(Netplayer::Player(1)));
    if let Some(winner) = winner {
        duel.first_attack_decider = Some(winner);
        duel.sender.send(stoc::SelectTp.into(), SendTarget::Single(winner.into()));
        duel.stage = DuelStage::Firstgo;
    }
}

#[handler(ctos::TpResult)]
#[register_to(YGOPRO_HANDLERS)]
fn on_tp_result(duel: &mut Duel) {
    duel.stage = DuelStage::Dueling;
    duel.start_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|r| r.as_secs() as u32)
            .unwrap_or(0);
    duel.set_player_info(CorePlayer::FirstAttackPlayer,  duel.host_info.start_lp as i32, duel.host_info.start_hand as i32, duel.host_info.draw_count as i32);
    duel.set_player_info(CorePlayer::SecondAttackPlayer, duel.host_info.start_lp as i32, duel.host_info.start_hand as i32, duel.host_info.draw_count as i32);
    duel.queue_request_ex(ygopro::FirstShuffle);
    duel.queue_request_ex(ygopro::DuelInit);
}

#[handler(ygopro::DuelStart)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_duel_start(duel: &mut Duel) {
    let mut options = DuelOptions::empty();
    if duel.host_info.no_shuffle_deck { options.insert(DuelOptions::PseudoShuffle); }
    if duel.host_info.mode == Mode::Tag { options.insert(DuelOptions::TagMode); }
    duel.start(options, duel.host_info.duel_rule);
    duel.request_sender.send(crate::duel::Request::Evolve).ok();
}

#[handler(ctos::UpdateDeck)]
#[register_to(YGOPRO_HANDLERS)]
fn on_update_deck(duel: &mut Duel, player: PlayerIndex, update_deck: &ctos::UpdateDeck) -> Option<stoc::Message> {
    let netplayer: Netplayer = player.into();
    if duel.get(player)?.ready {
        warn!("UpdateDeck requested but player is already ready");
        return None;
    }
    let mut deck = update_deck.deck.clone();
    if duel.duel_count == 0 {
        let data_manager = data_manager::load_full();
        let player = duel.get_mut(player)?;
        player.deck_error = deck.load(|code| data_manager.get_card(code));
        player.deck = deck;
    } else {
        let data_manager = data_manager::load_full();
        let side_check_result = duel.get(player)?.deck.check_after_replacing_side(&mut deck, |code| data_manager.get_card(code));
        if let Err(_error) = side_check_result {
            return Some(stoc::ErrorMessage { err: ErrorMessage::SideError }.into());
        }
        if let Some(player) = duel.get_mut(player) {
            player.deck = deck;
            player.ready = true;
        }
        duel.sender.send(stoc::DuelStart.into(), netplayer.into());
        let ready = duel.players.iter().all(|p| p.as_ref().map(|p| p.ready) == Some(true));
        if ready {
            let decider = duel.first_attack_decider.unwrap_or(PlayerIndex::Player1);
            duel.sender.send(stoc::SelectTp.into(), decider.into());
            for (index, player) in duel.players.iter_mut().enumerate() {
                if let Some(player) = player.as_mut() {
                    if index == decider.0 as usize {
                        player.state = AllowMessage::Some(ctos::MessageType::TpResult)
                    } else { player.state = AllowMessage::None; }
                }
            }
            duel.stage = DuelStage::Firstgo; 
        }
    }
    None
}

#[handler(ctos::CreateGame)]
#[register_to(YGOPRO_HANDLERS)]
fn on_create_game(duel: &mut Duel, create_game: &ctos::CreateGame) {
    duel.host_info = create_game.host_info.clone();
    duel.name = create_game.name.clone();
    duel.pass = create_game.pass.clone();
}

#[handler(ctos::JoinGame)]
#[register_to(YGOPRO_HANDLERS)]
fn on_join_game(duel: &mut Duel, player: Netplayer, request: &mut Request, join_game: &ctos::JoinGame) -> Result<Vec<stoc::Message>, stoc::Message> {
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
    let player = match duel.uninit_players.try_remove(undecided_index) {
        Some(player) => player,
        None => { warn!("Cannot find uninit player: {:?}", player); return Err(stoc::ErrorMessage { err: ErrorMessage::JoinError(JoinError::HostRefused) }.into()); }
    };
    duel.sender.undecided.remove(undecided_index);
    let mut response_messages = vec![];

    // calculate current user position
    let is_creator = duel.players.iter().all(|p| p.is_none()) && duel.observers.is_empty();
    let mut observer_count = duel.observers.len() as u16;
    
    let pos = match duel.players.iter().position(|player| player.is_none()) {
        Some(index) => Netplayer::Player(index as u8),
        None => {
            observer_count += 1;
            Netplayer::Observer(duel.observers.vacant_entry().key() as u8)
        }
    };
    request.extra = pos;
    if is_creator { duel.host_player = pos; }

    let deck_manager = deck_manager::load();
    let mut join_info = duel.host_info.clone();
    if let Some(lflist) = deck_manager.get_lflist_by_index(join_info.lflist) {
        join_info.lflist = lflist.hash;
    }
    response_messages.push(stoc::JoinGame{ info: join_info }.into());
    response_messages.push(stoc::TypeChange{ 
        player: pos,
        host: is_creator
    }.into());
    
    // broadcast player change
    if matches!(pos, Netplayer::Observer(_)) {
        duel.sender.send(stoc::HsWatchChange { watch_count: observer_count }.into(), SendTarget::All);
    } else {
        duel.sender.send(stoc::HsPlayerEnter { name: player.name.clone(), pos }.into(), SendTarget::All);
    }

    let stoc_sender = player.stoc_sender.clone();
    match pos {
        Netplayer::Observer(_) => { duel.observers.insert(player); duel.sender.observers.insert(stoc_sender); }
        Netplayer::Player(index) => { duel.players[index as usize] = Some(player.into()); duel.sender.set_player(index as usize, stoc_sender); }
        _ => warn!("try to put into an illegal player pos")
    };

    // tell current user now how room is now.
    for (index, player) in duel.players.iter().enumerate() {
        if let Some(player) = player.as_ref() {
            let netplayer = Netplayer::Player(index as u8);
            response_messages.push(stoc::HsPlayerEnter { name: player.name.clone(), pos: netplayer }.into());
            if player.ready { response_messages.push(stoc::HsPlayerChange { status: PlayerChange::new()
                .with_player(netplayer)
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
fn on_hs_to_duelist(duel: &mut Duel, request: &mut Request, player: Netplayer) -> Option<stoc::Message> {
    let observer_index = if let Netplayer::Observer(observer_index) = player {
        observer_index as usize
    } else {
        warn!("HsToDuelist requested by non-observer");
        return None;
    };
    let new_position_index = match duel.players.iter().position(|player| player.is_none()) {
        Some(index) => index,
        None => { warn!("HsToDuelist requested but all player slots are full"); return None; }
    };
    let Some(observer) = duel.observers.try_remove(observer_index) else {
        warn!("try to convert observer to player but observer dont exist");
        return None;
    };
    let stoc_sender = observer.stoc_sender.clone();
    duel.sender.observers.remove(observer_index);
    let i_am_host = duel.host_player == player;
    let new_position = Netplayer::Player(new_position_index as u8);
    request.extra = new_position;
    if i_am_host { duel.host_player = new_position; }
    let name = observer.name.clone();
    duel.players[new_position_index] = Some(observer.into());
    duel.sender.set_player(new_position_index, stoc_sender);
    duel.sender.send(stoc::HsPlayerEnter { name, pos: new_position }.into(), SendTarget::All);
    let watch_count = duel.observers.len() as u16;
    duel.sender.send(stoc::HsWatchChange { watch_count }.into(), SendTarget::All);
    Some(stoc::TypeChange {
        player: new_position,
        host: i_am_host
    }.into())
}

#[handler(ctos::HsToObserver)]
#[register_to(YGOPRO_HANDLERS)]
fn on_hs_to_observer(duel: &mut Duel, request: &mut Request, player: PlayerIndex) -> Option<stoc::Message> {
    let original_netplayer: Netplayer = player.into();
    let position = player.0 as usize;
    let Some(duel_player) = duel.players[position].take() else {
        warn!("to_observer requested but player slot is empty");
        return None;
    };
    let stoc_sender = duel_player.player.stoc_sender.clone();
    duel.sender.clear_player(position);
    let current_netplayer = Netplayer::Observer(duel.observers.insert(duel_player.player) as u8);
    duel.sender.observers.insert(stoc_sender);
    request.extra = current_netplayer;
    let i_am_host = duel.host_player == original_netplayer;
    if i_am_host { duel.host_player = current_netplayer }
    duel.sender.send(stoc::HsPlayerChange { 
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
fn on_leave_game(duel: &mut Duel, player: Netplayer) -> bool {
    if player == duel.host_player {
        let new_host: Netplayer = match duel.players.iter().enumerate()
            .find(|(index, slot)| slot.is_some() && Netplayer::Player(*index as u8) != player)
            .map(|(index, _)| Netplayer::Player(index as u8)) {
            Some(candidate) => candidate,
            None => { duel.end(); return true; }
        };
        duel.host_player = new_host;
        if duel.stage == DuelStage::Begin {
            if let Some(player) = duel.get_net_mut(new_host) {
                player.ready = false;
            }
            duel.sender.send(stoc::TypeChange {
                player: new_host,
                host: true
            }.into(), SendTarget::Single(new_host));
        }
    }

    match player {
        Netplayer::Observer(observer_index) => {
            let index = observer_index as usize;
            if duel.observers.try_remove(index).is_none() {
                warn!("LeaveGame requested by unknown observer");
            } else {
                duel.sender.observers.remove(index);
                if duel.stage == DuelStage::Begin {
                    let watch_count = duel.observers.len() as u16;
                    duel.sender.send(stoc::HsWatchChange { watch_count }.into(), SendTarget::All);
                }
            }
        }
        Netplayer::Player(leaving_netplayer) => {
            if duel.stage == DuelStage::Begin {
                duel.players[leaving_netplayer as usize] = None;
                duel.sender.clear_player(leaving_netplayer as usize);
                let leave_message: stoc::Message = stoc::HsPlayerChange { status: PlayerChange::new()
                    .with_state(PlayerChangeState::Leave)
                    .with_player(player)
                }.into();
                duel.sender.send(leave_message, SendTarget::All);
            } else {
                // if duel.stage == DuelStage::Siding {
                //     duel.sender.send(stoc::DuelStart.into(), SendTarget::AllPlayer);
                // }
                // // a leave after the duel already ended must not announce a second win.
                // if duel.stage != DuelStage::End && !duel.ended {
                //     let leaving_index = PlayerIndex::from(leaving_netplayer);
                //     let loser = duel.to_core_player(leaving_index);
                //     duel.sender.send(gm::Win { winner: loser.opponent(), reason: WinReason::OpponentLeave }.into(), SendTarget::All);
                //     duel.queue_request_ex(ygopro::DuelEnd { winner: loser.opponent(), reason: WinReason::OpponentLeave });
                // }
            }
            duel.players[leaving_netplayer as usize] = None;
            duel.sender.clear_player(leaving_netplayer as usize);
        }
        Netplayer::Undecided(leaving_player) => {
            if duel.uninit_players.try_remove(leaving_player as usize).is_none() {
                warn!("LeaveGame requested by unknown undecided player");
            } else {
                duel.sender.undecided.remove(leaving_player as usize);
            }
        }
        Netplayer::Unknown => {}
    }
    duel.players.iter().all(|player| player.is_none()) && duel.observers.is_empty()
}

#[handler(ctos::HsStart)]
#[register_to(YGOPRO_HANDLERS)]
fn on_hs_start(duel: &mut Duel, player: Netplayer) {
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
    duel.sender.send(stoc::DuelStart.into(), SendTarget::All);
    for (_, observer) in duel.observers.iter_mut() {
        observer.state = AllowMessage::None;
    }

    let player1_count = stoc::DeckCount {
        mainc_s: deck1_main, sidec_s: deck1_side, extrac_s: deck1_extra,
        mainc_o: deck2_main, sidec_o: deck2_side, extrac_o: deck2_extra,
    };
    let player2_count = stoc::DeckCount {
        mainc_s: deck2_main, sidec_s: deck2_side, extrac_s: deck2_extra,
        mainc_o: deck1_main, sidec_o: deck1_side, extrac_o: deck1_extra,
    };
    duel.sender.send(player1_count.into(), SendTarget::Single(Netplayer::Player(0)));
    duel.sender.send(player2_count.into(), SendTarget::Single(Netplayer::Player(1)));

    duel.sender.send(stoc::SelectHand.into(), SendTarget::AllPlayer);

    let (player1, player2) = duel.players.split_at_mut(1);
    if let (Some(player1), Some(player2)) = (player1[0].as_mut(), player2[0].as_mut()) {
        player1.hand = None;
        player2.hand = None;
        player1.state = AllowMessage::Some(ctos::MessageType::HandResult);
        player2.state = AllowMessage::Some(ctos::MessageType::HandResult);
    } else {
        warn!("HsStart: one of the players is missing");
    }
}

#[handler(ctos::PlayerInfo)]
#[register_to(YGOPRO_HANDLERS)]
fn on_player_info(duel: &mut Duel, player: Netplayer, player_info: &ctos::PlayerInfo) {
    let index = match player {
        Netplayer::Undecided(index) => index,
        _ => { warn!("A decided player {:?} sent player info.", player); return } 
    };
    if let Some(player) = duel.uninit_players.get_mut(index as usize) {
        player.name = player_info.name.clone();
    } else {
        warn!("We receive a player_info, but no user is waiiting init.");
    }
}

#[handler(ctos::HsReady)]
#[register_to(YGOPRO_HANDLERS)]
fn on_hs_ready(duel: &mut Duel, index: PlayerIndex) -> Vec<stoc::Message> {
    let netplayer: Netplayer = index.into();
    if duel.stage != DuelStage::Begin {
        warn!("HsReady requested outside Begin stage");
        return vec![];
    }
    let no_check_deck = duel.host_info.no_check_deck;
    let lflist_hash = duel.host_info.lflist;
    let rule = duel.host_info.rule;
    let Some(duel_player) = duel.get_mut(index) else {
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
        let data_manager = data_manager::load_full();
        let lflist = deck_manager.get_lflist_by_hash(lflist_hash).cloned().unwrap_or_else(|| ygopro_data::data::LFList::new(String::new()));
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
        duel.sender.send(stoc::HsPlayerChange { status: PlayerChange::new().with_state(PlayerChangeState::Ready).with_player(netplayer)}.into(), SendTarget::All);
    }
    messages
}

#[handler(ctos::HsNotReady)]
#[register_to(YGOPRO_HANDLERS)]
fn on_hs_not_ready(duel: &mut Duel, index: PlayerIndex) {
    if duel.stage != DuelStage::Begin { 
        warn!("HsNotReady requested outside Begin stage"); 
        return; 
    }
    let Some(duel_player) = duel.get_mut(index) else {
        warn!("HsNotReady requested by non-player");
        return;
    };
    if !duel_player.ready { 
        warn!("HsNotReady requested but player is already not ready"); 
        return 
    }
    duel_player.ready = false;
    let netplayer: Netplayer = index.into();
    duel.sender.send(stoc::HsPlayerChange { 
        status: PlayerChange::new()
            .with_state(PlayerChangeState::Notready)
            .with_player(netplayer) 
    }.into(), SendTarget::All);
}

#[handler(ctos::HsKick)]
#[register_to(YGOPRO_HANDLERS)]
fn on_hs_kick(duel: &mut Duel, kicker: Netplayer, kick: &ctos::HsKick) {
    if kicker != duel.host_player { warn!("HsKick requested by non-host"); return; }
    if duel.stage != DuelStage::Begin { warn!("HsKick requested outside Begin stage"); return; }
    let Netplayer::Player(target) = kick.pos else {
        warn!("HsKick requested to kick non-player");
        return;
    };
    if kicker == kick.pos { warn!("HsKick: cannot kick self"); return; }
    if duel.players[target as usize].is_none() { warn!("HsKick: target slot empty"); return; }
    duel.sender.clear_player(target as usize);
    duel.sender.send(stoc::LeaveGame { pos: kick.pos }.into(), kick.pos.into());
    duel.players[target as usize].take();
    duel.sender.send(stoc::HsPlayerChange {
        status: PlayerChange::new()
            .with_state(PlayerChangeState::Leave)
            .with_player(kick.pos)
    }.into(), SendTarget::Except(kick.pos));
}

#[handler(ygopro::ClientJoin)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_client_join(duel: &mut Duel, join: &mut ygopro::ClientJoin) {
    if duel.stage > DuelStage::Begin { return }
    if let Some(oneshot) = join.position_sender.take() {
        let player = crate::player::BaseDuelPlayer::new(join.stoc_sender.clone());
        let undecided_index = duel.uninit_players.insert(player);
        duel.sender.undecided.insert(join.stoc_sender.clone());
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

// #[handler(ygopro::GenerateReplay)]
// #[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
// fn on_generate_replay(duel: &mut Duel) -> Option<stoc::Message> {
//     let mut replay = duel.create_replay_without_data()?;
//     replay.body.datas = duel.client_responses.clone().into_iter().map(|r| r.response.into()).collect();
//     Some(stoc::Replay { replay: Box::new(replay) }.into())
// }

#[handler(ygopro::DuelEnd)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_duel_end(duel: &mut Duel) {
    duel.core.end();
    duel.duel_count += 1;
}



#[handler(ygopro::JudgeContinueMatch, priority = 250)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_judge_continue_match_kill(duel: &mut Duel) -> &'static str {
    if duel.match_kill_card_code > 0 { "terminate" } else { "continue" }
}

#[handler(ygopro::JudgeContinueMatch, priority = 251)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_judge_continue_somebody_leave(duel: &mut Duel) -> &'static str {
    if duel.players.iter().any(|p| p.is_none()) { "terminate" } else { "continue" }
}

#[handler(ygopro::MatchEnd)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_match_end(duel: &mut Duel) {
    duel.sender.send(stoc::DuelEnd.into(), SendTarget::All);
}

#[handler(ygopro::RecreateDuel)]
#[register_to(YGOPRO_HANDLERS_EX as HandlerEx)]
fn on_recreate_duel(duel: &mut Duel) {
    for player in &mut duel.players {
        if let Some(player) = player.as_mut() { 
            player.state = AllowMessage::Some(ctos::MessageType::UpdateDeck);
            player.ready = false;
        }
    }
    duel.stage = DuelStage::Siding;
    duel.sender.send(stoc::ChangeSide.into(), SendTarget::AllPlayer);
    duel.sender.send(stoc::WaitingSide.into(), SendTarget::AllObserver);
    duel.core = ygopro_core_wrapper::Duel::new(duel.configuration.seed(duel.duel_count));
    duel.client_responses.clear();
}
