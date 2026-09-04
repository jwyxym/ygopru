use std::io::Cursor;
use std::mem::MaybeUninit;
use std::ops::Deref;
use std::ops::DerefMut;

use binrw::BinRead;
use bytes::BytesMut;
use slab::Slab;
use tokio::sync::mpsc;

use ygopro_core_wrapper as core;
use ygopro_data::complex::Complex;
use ygopro_data::constants::*;
use ygopro_data::data::CardPosition;
use ygopro_data::data::UpdateCardInfo;
use ygopro_data::message::HostInfo;
use ygopro_data::message::gm::GameMessage;
use ygopro_data::message::gm::MaskedClone;
use ygopro_data::message::{ctos, stoc, gm};
use ygopro_data::message::gm::CardCode;
use ygopro_data::string::FixedLengthString;
use ygopro_handler::FromRequest;

use crate::Configuration;
use crate::ygopro_handlers;

pub(crate) fn log_plugin_statistics(
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

#[derive(Clone, Copy, Default)]
pub enum SendTarget {
    Single(Netplayer),
    Except(Netplayer),
    Core(CorePlayer),
    #[default]
    All,
    AllPlayer,
    AllObserver,
    None
}

impl From<Netplayer> for SendTarget {
    fn from(value: Netplayer) -> Self {
        SendTarget::Single(value)
    }
}

impl From<CorePlayer> for SendTarget {
    fn from(value: CorePlayer) -> Self {
        SendTarget::Core(value)
    }
}

impl<Message, State, Res> FromRequest<ygopro_handler::extract::Request<Message, SendTarget>, State, Res> for &mut SendTarget
where State: Send, Res: Send, Message: Send {
    fn from_request(bundle: &mut ygopro_handler::Bundle<ygopro_handler::extract::Request<Message, SendTarget>, State, Res>) -> Option<Self> {
        Some(unsafe { &mut *(&mut bundle.request.extra as *mut SendTarget) })
    }
}

pub trait SendableMessage {
    fn resolve(&self, player_index: Netplayer) -> Complex<stoc::Message>;
    fn into_inner(self, player_index: Netplayer) -> Complex<stoc::Message>;
}

impl SendableMessage for Complex<stoc::Message> {
    fn resolve(&self, _: Netplayer) -> Complex<stoc::Message> {
        self.clone()
    }
    
    fn into_inner(self, _: Netplayer) -> Complex<stoc::Message> {
        self
    }
}

struct MaymaskedMessage<F> {
    pub message: Complex<stoc::Message>,
    pub masked_message: Complex<stoc::Message>,
    pub mask_judger: F
}

impl<F> SendableMessage for MaymaskedMessage<F> where F: Fn(Netplayer) -> bool {
    fn resolve(&self, player_index: Netplayer) -> Complex<stoc::Message> {
        if (self.mask_judger)(player_index) {
            self.message.clone()
        } else {
            self.masked_message.clone()
        }
    }
    
    fn into_inner(self, player: Netplayer) -> Complex<stoc::Message> {
        if (self.mask_judger)(player) {
            self.message
        } else {
            self.masked_message
        }
    }
}

impl<F> MaymaskedMessage<F> {
    fn new(message: gm::Message, f: F) -> Self {
        let masked_message = message.clone_masked();
        Self {
            message: Complex::from_message(stoc::Message::from(message)),
            masked_message: Complex::from_message(stoc::Message::from(masked_message)),
            mask_judger: f,
        }
    }
}

pub trait CorePlayerToSendTarget {
    fn transform(&self, player: CorePlayer) -> SendTarget;
}

pub struct Sender {
    pub players: Vec<mpsc::UnboundedSender<Complex<stoc::Message>>>,
    pub observers: Slab<mpsc::UnboundedSender<Complex<stoc::Message>>>,
    pub undecided: Slab<mpsc::UnboundedSender<Complex<stoc::Message>>>, 
    pub messages: Vec<Complex<stoc::Message>>,
    pub masked_messages: Vec<Complex<stoc::Message>>
}

impl Sender {
    pub fn new() -> Self {
        Self {
            players: Vec::new(),
            observers: Slab::new(),
            undecided: Slab::new(),
            messages: Vec::new(),
            masked_messages: Vec::new(),
        }
    }

    pub fn send(&mut self, message: stoc::Message, target: SendTarget) {
        let complex_message = Complex::from_message(message);
        self.messages.push(complex_message.clone());
        self.masked_messages.push(complex_message.clone());
        self.send_without_record(complex_message, target);
    }

    pub fn send_game_message(&mut self, message: gm::Message, target: SendTarget, mask_judger: impl Fn(Netplayer) -> bool, core_transformer: impl CorePlayerToSendTarget) {
        let mut target = target;
        match target {
            SendTarget::Core(player) => target = core_transformer.transform(player),
            _ => (),
        }
        let is_waiting_for = message.waiting_for();
        let maymasked = MaymaskedMessage::new(message, mask_judger);
        self.messages.push(maymasked.message.clone());
        if is_waiting_for.is_none() {
            self.masked_messages.push(maymasked.masked_message.clone());
        }
        self.send_without_record(maymasked, target);
    }

    pub(crate) fn send_without_record(&self, message: impl SendableMessage, target: SendTarget) {
        match target {
            SendTarget::Single(netplayer) => match netplayer {
                Netplayer::Player(index) => Sender::_send_single(message.into_inner(netplayer), self.players.get(index as usize)),
                Netplayer::Observer(index) => Sender::_send_single(message.into_inner(netplayer), self.observers.get(index as usize)),
                Netplayer::Undecided(index) => Sender::_send_single(message.into_inner(netplayer), self.undecided.get(index as usize)),
                Netplayer::Unknown => log::warn!("Try to send a message to unknown."),
            },
            SendTarget::Except(netplayer) => {
                let players = self.players.iter().enumerate()
                    .filter(|(index, _)| !matches!(netplayer, Netplayer::Player(i) if i as usize == *index))
                    .map(|(index, sender)| (Netplayer::Player(index as u8), sender));
                let observers = self.observers.iter()
                    .filter(|(index, _)| !matches!(netplayer, Netplayer::Observer(i) if i as usize == *index))
                    .map(|(index, sender)| (Netplayer::Observer(index as u8), sender));
                Sender::_send_iter(&message, players.chain(observers));
            }
            SendTarget::Core(_) => { log::warn!("Try to send message to a core player without transforming") },
            SendTarget::All => Sender::_send_iter(&message,
                self.players.iter().enumerate()
                    .map(|(index, sender)| (Netplayer::Player(index as u8), sender))
                    .chain(self.observers.iter()
                        .map(|(index, sender)| (Netplayer::Observer(index as u8), sender))),
            ),
            SendTarget::AllPlayer => Sender::_send_iter(&message,
                self.players.iter().enumerate()
                    .map(|(index, sender)| (Netplayer::Player(index as u8), sender)),
            ),
            SendTarget::AllObserver => Sender::_send_iter(&message,
                self.observers.iter()
                    .map(|(index, sender)| (Netplayer::Observer(index as u8), sender)),
            ),
            SendTarget::None => (),
        }
    }

    fn _send_iter<'a>(message: &impl SendableMessage, iter: impl Iterator<Item = (Netplayer, &'a mpsc::UnboundedSender<Complex<stoc::Message>>)>) {
        for (netplayer, target) in iter {
            Sender::_send(message.resolve(netplayer), target);
        }
    }

    fn _send_single(message: Complex<stoc::Message>, target: Option<&mpsc::UnboundedSender<Complex<stoc::Message>>>) {
        if let Some(target) = target {
            Sender::_send(message, target);
        }
    }

    fn _send(message: Complex<stoc::Message>, target: &mpsc::UnboundedSender<Complex<stoc::Message>>) {
        target.send(message).ok();
    }
}

impl Sender {
    pub(crate) fn set_player(&mut self, index: usize, sender: mpsc::UnboundedSender<Complex<stoc::Message>>) {
        if self.players.len() <= index { self.players.resize(index + 1, Self::dummy_sender()); }
        self.players[index] = sender;
    }

    pub(crate) fn clear_player(&mut self, index: usize) {
        if self.players.len() <= index { self.players.resize(index + 1, Self::dummy_sender()); }
        self.players[index] = Self::dummy_sender();
    }

    fn dummy_sender() -> mpsc::UnboundedSender<Complex<stoc::Message>> {
        let (sender, _receiver) = mpsc::unbounded_channel();
        sender
    }
}


pub enum Request {
    Message(ygopro_handlers::Request),
    MessageEx(ygopro_handlers::RequestEx),
    Evolve,
    Command { name: &'static str, arguments: [u8; 8] }
}

type BaseDuelPlayer = crate::player::BaseDuelPlayer<Complex<stoc::Message>>;
type DuelPlayer = crate::player::DuelPlayer<Complex<stoc::Message>>;

#[derive(Copy, Clone, Eq, PartialEq, Debug, PartialOrd, Ord, Hash)]
pub struct PlayerIndex(pub u8);

impl PlayerIndex {
    pub const Player1: PlayerIndex = PlayerIndex(0);
    pub const Player2: PlayerIndex = PlayerIndex(1);
    pub const Player3: PlayerIndex = PlayerIndex(2);
    pub const Player4: PlayerIndex = PlayerIndex(3);
}

impl From<PlayerIndex> for Netplayer {
    fn from(value: PlayerIndex) -> Self {
        Netplayer::Player(value.0)
    }
}

impl TryFrom<Netplayer> for PlayerIndex {
    type Error = ();

    fn try_from(value: Netplayer) -> Result<Self, Self::Error> {
        match value {
            Netplayer::Player(index) => Ok(PlayerIndex(index)),
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

impl From<u8> for PlayerIndex {
    fn from(value: u8) -> Self {
        PlayerIndex(value)
    }
}

pub struct Duel {
    pub core: core::Duel,
    pub name: FixedLengthString<20>,
    pub pass: FixedLengthString<20>,
    pub host_player: Netplayer,
    pub host_info: HostInfo,
    pub stage: DuelStage,
    pub sender: Sender,
    pub response_buffer: BytesMut,
    pub core_request_buffer: BytesMut,
    pub players: Vec<Option<DuelPlayer>>,
    pub max_player_count: usize,
    pub observers: Slab<BaseDuelPlayer>,
    pub match_kill_card_code: i32,
    pub duel_count: u8,
    pub first_attack_decider: Option<PlayerIndex>,
    pub last_select_message: Option<gm::Message>,
    pub last_response: Option<PlayerIndex>,
    // extended by rust ygopro
    pub configuration: Configuration,
    pub uninit_players: Slab<BaseDuelPlayer>,
    // replay recorder
    pub start_time: u32,
    pub client_responses: Vec<ctos::Response>,
    // extended by actor models
    pub request_sender: mpsc::UnboundedSender<Request>,
    pub(crate) request_receiver: Option<mpsc::UnboundedReceiver<Request>>,
}

impl Duel {
    pub fn new(host_info: HostInfo, mut configuration: Configuration) -> Self {
        let seed = configuration.seed(0);
        let (request_sender, request_receiver) = mpsc::unbounded_channel();
        Self {
            host_player: Netplayer::Unknown,
            host_info,
            stage: DuelStage::Begin,
            core: core::Duel::new(seed),
            name: FixedLengthString::allocate(),
            pass: FixedLengthString::allocate(),
            sender: Sender::new(),
            response_buffer: BytesMut::zeroed(core::SIZE_RETURN_VALUE),
            core_request_buffer: BytesMut::zeroed(core::SIZE_QUERY_BUFFER),
            players: Vec::new(),
            max_player_count: 0,
            observers: Slab::new(),
            match_kill_card_code: -1,
            duel_count: 0,
            first_attack_decider: None,
            last_select_message: None,
            last_response: None,
            configuration,
            uninit_players: Slab::new(),
            start_time: 0,
            client_responses: Vec::new(),
            request_sender,
            request_receiver: Some(request_receiver),
        }
    }

    pub fn get(&self, index: PlayerIndex) -> Option<&DuelPlayer> {
        self.players.get(index.0 as usize)?.as_ref()
    }

    pub fn get_net(&self, index: Netplayer) -> Option<&DuelPlayer> {
        let Netplayer::Player(index) = index else { return None };
        self.players.get(index as usize)?.as_ref()
    }

    pub fn get_mut(&mut self, index: PlayerIndex) -> Option<&mut DuelPlayer> {
        self.players.get_mut(index.0 as usize)?.as_mut()
    }

    pub fn get_net_mut(&mut self, index: Netplayer) -> Option<&mut DuelPlayer> {
        let Netplayer::Player(index) = index else { return None };
        self.players.get_mut(index as usize)?.as_mut()
    }

    pub fn get_many_mut<const L: usize>(&mut self, index: [PlayerIndex; L]) -> [&mut Option<DuelPlayer>; L] {
        let player_count = self.players.len();
        let players_ptr = self.players.as_mut_ptr();
        let mut result: [MaybeUninit<&mut Option<DuelPlayer>>; L] = std::array::from_fn(|_| MaybeUninit::uninit());
        for (slot, player_index) in index.iter().copied().enumerate() {
            let position = player_index.0 as usize;
            assert!(position < player_count, "player index out of bounds");
            assert!(!index[..slot].contains(&player_index), "duplicate player index");
            unsafe {
                result[slot].write(&mut *players_ptr.add(position));
            }
        }
        unsafe { result.map(|slot| slot.assume_init()) }
    }

    pub fn queue_request<Message: Into<ctos::Message>>(&self, message: Message, player: Netplayer) {
        self.request_sender.send(Request::Message( ygopro_handlers::Request { message: message.into(), extra: player } )).ok();
    }

    pub fn queue_request_ex<Message: Into<crate::message::Message>>(&self, message: Message) {
        self.request_sender.send(Request::MessageEx( ygopro_handlers::RequestEx { message: message.into(), extra: SendTarget::All } )).ok();
    }

    pub fn queue_command(&self, command: &'static str, args: [u8; 8]) {
        self.request_sender.send(Request::Command { name: command, arguments: args }).ok();
    }

}

impl Deref for Duel {
    type Target = core::Duel;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl DerefMut for Duel {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl Duel {
    pub fn default_query(location: Location) -> Query {
        let every_one_want_this_query: Query = Query::Code | Query::Position | Query::Alias | Query::Type
                                                | Query::Level | Query::Rank | Query::Attribute | Query::Race
                                                | Query::Attack | Query::Defense | Query::BaseAttack | Query::BaseDefense
                                                | Query::Reason;
        match location {
            Location::Extra => Query::Link | Query::LeftScale | Query::RightScale | Query::Status | every_one_want_this_query,
            Location::Grave | Location::Removed => Query::Status | every_one_want_this_query,
            Location::Hand | Location::SZone => Query::LeftScale | Query::RightScale | Query::Status | every_one_want_this_query,
            Location::MZone => Query::Link | Query::Status | every_one_want_this_query,
            _ => Query::all()
        }
    }

    pub fn query_location_cards(&mut self, player: CorePlayer, location: Location, query: Query) -> gm::Message {
        let data_size = self.core.query_field_card(player, location, query, &mut self.core_request_buffer, false) as usize;
        let mut cursor = Cursor::new(&self.core_request_buffer[..data_size]);
        let cards: Vec<UpdateCardInfo> = (0..).map_while(|_| UpdateCardInfo::read_le(&mut cursor).ok()).collect();
        gm::UpdateData { player, location, data: cards }.into()
    }

    pub fn refresh_location(&mut self, player: CorePlayer, locations: Location, query: Query) -> Vec<gm::Message> {
        let mut messasges = vec![];
        let players: &[CorePlayer] = if player == CorePlayer::All {
            &[CorePlayer::FirstAttackPlayer, CorePlayer::SecondAttackPlayer]
        } else {
            std::slice::from_ref(&player)
        };
        for &player in players {
            for location in Location::iter(&locations) {
                let query = if query.is_empty() { Self::default_query(location) } else { query };
                messasges.push(self.query_location_cards(player, location, query));
            }
        }
        messasges
    }

    pub fn refresh_card(&mut self, player: CorePlayer, location: Location, sequence: i8, mut query: Query) -> gm::Message {
        if query.is_empty() { query = Query::from_bits_retain(0xf81fff); }
        let len = self.core.query_card(player, location, sequence as u8, query, &mut self.core_request_buffer, false) as usize;
        let mut cursor = Cursor::new(&self.core_request_buffer[..len]);
        let card = UpdateCardInfo::read_le(&mut cursor).ok().unwrap_or(UpdateCardInfo::Empty);
        gm::UpdateCard { 
            position: CardPosition::<false, false, false> { 
                code: CardCode::new(),
                controller: player,
                location,
                sequence: sequence as i8,
                sub_sequence: 0,
                description: 0,
            },
            data: card,
        }.into()
    }

    pub fn send_game_message(&mut self, message: gm::Message, target: SendTarget, core_transformer: impl CorePlayerToSendTarget + PlayerConverter) {
        if message.waiting_for().is_some() { self.last_select_message = Some(message.clone()); }
        if self.configuration.no_mask {
            self.sender.send_game_message(message, target, |_| false, core_transformer);
        } else {
            let can_player_see_unmasked: Vec<bool> = (0u8..(self.max_player_count as u8)).map(|index| !message.should_mask(core_transformer.to_core_player(Netplayer::Player(index)))).collect();
            let mask_judger = move |netplayer: Netplayer| matches!(netplayer, Netplayer::Player(index) if can_player_see_unmasked[index as usize]);
            self.sender.send_game_message(message, target, mask_judger, core_transformer);
        }
    }

    pub fn refresh(&mut self, player: CorePlayer, locations: Location, sequence: i8, query: Query, core_transformer: impl CorePlayerToSendTarget + PlayerConverter + Clone) {
        if sequence >= 0 {
            let message = self.refresh_card(player, locations, sequence, query);
            self.send_game_message(message, SendTarget::All, core_transformer.clone());
        } else {
            for message in self.refresh_location(player, locations, query).into_iter() {
                self.send_game_message(message, SendTarget::All, core_transformer.clone());
            }
        }
    }
}

pub fn response_is_meaningful(response: &ygopro_data::data::Response, last_select_message: &gm::Message) -> bool {
    if gm::MessageType::from(last_select_message) == gm::MessageType::Retry { return false; }
    let resolved = match response {
        ygopro_data::data::Response::Unknown(data) => {
            let mut resolved = ygopro_data::data::Response::Unknown(data.clone());
            resolved.resolve(gm::MessageType::from(last_select_message)).ok();
            Some(resolved)
        }
        _ => None,
    };
    let response = resolved.as_ref().unwrap_or(response);
    match response {
        ygopro_data::data::Response::Cancel => gm::MessageType::from(last_select_message) != gm::MessageType::SelectUnselectCard,
        ygopro_data::data::Response::DeclineChain | ygopro_data::data::Response::SelectUnselectCards(_) => false,
        ygopro_data::data::Response::SelectIdleCommand(command, _) => (*command) as u16 <= ygopro_data::data::IdleCommand::Activate as u16,
        ygopro_data::data::Response::SelectBattleCommand(command, _) => (*command) as u16 <= ygopro_data::data::BattleCommand::Attack as u16,
        _ => true,
    }
}
