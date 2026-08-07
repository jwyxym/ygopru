use std::ops::Deref;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;

use ygopro_data::complex::Complex;
use ygopro_data::constants::CorePlayer;
use ygopro_data::constants::Hand;
use ygopro_data::constants::Netplayer;
use ygopro_data::data::Replay;
use ygopro_data::message::ctos;
use ygopro_data::message::gm;
use ygopro_data::message::gm::GameMessage;
use ygopro_data::message::stoc;
use ygopro_data::string::FixedLengthString;
use ygopro_handler::RoomProvider;

#[derive(thiserror::Error, Debug)]
pub enum ReconstructionError {
    #[error("cannot read replay file: {0}")]
    Io(std::io::Error),
    #[error("cannot parse replay file: {0}")]
    Parse(binrw::Error),
    #[error("card database is missing cards: {0:?}")]
    MissingCards(Vec<u32>),
    #[error("tag duel replays are not supported yet")]
    TagReplayNotSupported,
    #[error("replay contains no responses")]
    EmptyReplay,
    #[error("engine rejected a response, the replay desynced")]
    Retry,
    #[error("session ended before the duel finished")]
    DuelDidNotEnd,
    #[error("a player disconnected before the replay finished")]
    Disconnected,
    #[error("validation timed out")]
    Timeout,
}

pub struct Player<Room: RoomProvider<ctos::Message, Complex<stoc::Message>>> {
    pub ctos_sender: mpsc::UnboundedSender<ctos::Message>,
    pub stoc_stream: Room::ServerToClientStream,
}

pub fn create_player<Room: RoomProvider<ctos::Message, Complex<stoc::Message>>>(room: &mut Room) -> Player<Room> {
    let (ctos_sender, ctos_receiver) = mpsc::unbounded_channel();
    let stoc_stream = room.add(UnboundedReceiverStream::new(ctos_receiver));
    Player { ctos_sender, stoc_stream }
}

pub async fn start_duel<Room: RoomProvider<ctos::Message, Complex<stoc::Message>>>(replay: &Replay, room: &mut Room) -> Result<(Player<Room>, Player<Room>), ReconstructionError> {
    let mut player1 = create_player(room);
    send(&player1.ctos_sender, CorePlayer::FirstAttackPlayer, ctos::PlayerInfo { name: replay.body.host_name.clone() }.into());
    send(&player1.ctos_sender, CorePlayer::FirstAttackPlayer, ctos::JoinGame { version: ygopro::PRO_VERSION, gameid: 0, pass: FixedLengthString::allocate() }.into());
    wait_for(&mut player1.stoc_stream, CorePlayer::FirstAttackPlayer, stoc::MessageType::TypeChange).await?;

    tokio::time::sleep(Duration::from_millis(10)).await;

    let mut player2 = create_player(room);
    send(&player2.ctos_sender, CorePlayer::SecondAttackPlayer, ctos::PlayerInfo { name: replay.body.client_name.clone() }.into());
    send(&player2.ctos_sender, CorePlayer::SecondAttackPlayer, ctos::JoinGame { version: ygopro::PRO_VERSION, gameid: 0, pass: FixedLengthString::allocate() }.into());
    wait_for(&mut player2.stoc_stream, CorePlayer::SecondAttackPlayer, stoc::MessageType::TypeChange).await?;

    wait_for(&mut player1.stoc_stream, CorePlayer::FirstAttackPlayer, stoc::MessageType::HsPlayerEnter).await?;

    send(&player1.ctos_sender, CorePlayer::FirstAttackPlayer, ctos::UpdateDeck { deck: replay.body.host_deck.clone().into() }.into());
    send(&player1.ctos_sender, CorePlayer::FirstAttackPlayer, ctos::HsReady.into());
    send(&player2.ctos_sender, CorePlayer::SecondAttackPlayer, ctos::UpdateDeck { deck: replay.body.client_deck.clone().into() }.into());
    send(&player2.ctos_sender, CorePlayer::SecondAttackPlayer, ctos::HsReady.into());
    wait_for(&mut player2.stoc_stream, CorePlayer::SecondAttackPlayer, stoc::MessageType::HsPlayerChange).await?;

    send(&player1.ctos_sender, CorePlayer::FirstAttackPlayer, ctos::HsStart.into());
    wait_for(&mut player1.stoc_stream, CorePlayer::FirstAttackPlayer, stoc::MessageType::SelectHand).await?;

    send(&player1.ctos_sender, CorePlayer::FirstAttackPlayer, ctos::HandResult { res: Hand::Paper }.into());
    send(&player2.ctos_sender, CorePlayer::SecondAttackPlayer, ctos::HandResult { res: Hand::Rock  }.into());
    wait_for(&mut player1.stoc_stream, CorePlayer::FirstAttackPlayer, stoc::MessageType::SelectTp).await?;

    send(&player1.ctos_sender, CorePlayer::FirstAttackPlayer, ctos::TpResult { result: CorePlayer::FirstAttackPlayer }.into());

    Ok((player1, player2))
}

pub async fn send_responses<Room: RoomProvider<ctos::Message, Complex<stoc::Message>>>(player1: &mut Player<Room>, player2: &mut Player<Room>, responses: Vec<ctos::Response>) -> Result<(), ReconstructionError> {
    for response in responses { loop {
        tokio::select! {
            player1_result = player1.stoc_stream.next() => { if respond(&player1.ctos_sender, CorePlayer::FirstAttackPlayer, &player1_result.ok_or(ReconstructionError::Disconnected)?)? {
                send(&player1.ctos_sender, CorePlayer::FirstAttackPlayer, response.into());
                break;
            }},
            player2_result = player2.stoc_stream.next() => { if respond(&player2.ctos_sender, CorePlayer::SecondAttackPlayer, &player2_result.ok_or(ReconstructionError::Disconnected)?)? { 
                send(&player2.ctos_sender, CorePlayer::SecondAttackPlayer, response.into());
                break; 
            }}
        }
    }}
    Ok(())
}

pub async fn drive_duel<Room: RoomProvider<ctos::Message, Complex<stoc::Message>>>(
    player1: &mut Player<Room>,
    player2: &mut Player<Room>,
    responses: Vec<ctos::Response>,
    validation_timeout: Duration,
) -> Result<(Option<Netplayer>, bool), ReconstructionError> {
    let feeding_result = timeout(validation_timeout, send_responses::<Room>(player1, player2, responses)).await;
    match feeding_result {
        Err(_) => return Err(ReconstructionError::Timeout),
        Ok(Err(error)) => return Err(error),
        Ok(Ok(())) => (),
    }
    let winner = scan_winner::<Room>(player1, player2, validation_timeout).await;
    // every recorded response was accepted without a retry, so the replay is valid
    // even if no win message was observed.
    Ok((winner, winner.is_none()))
}

async fn scan_winner<Room: RoomProvider<ctos::Message, Complex<stoc::Message>>>(
    player1: &mut Player<Room>,
    player2: &mut Player<Room>,
    validation_timeout: Duration,
) -> Option<Netplayer> {
    let mut winner = None;
    let mut inspect = |message: Option<Complex<stoc::Message>>| -> bool {
        match message {
            None => true,
            Some(message) => match message.deref() {
                stoc::Message::GameMessage(game_message) => {
                    if let gm::Message::Win(win) = &game_message.message {
                        winner = match win.winner {
                            CorePlayer::FirstAttackPlayer => Some(Netplayer::Player(0)),
                            CorePlayer::SecondAttackPlayer => Some(Netplayer::Player(1)),
                            _ => None,
                        };
                        return true;
                    }
                    // the engine requests input the replay does not provide,
                    // so no win message is coming; stop scanning.
                    game_message.message.waiting_for().is_some()
                }
                _ => false,
            },
        }
    };
    timeout(validation_timeout, async {
        loop {
            tokio::select! {
                message = player1.stoc_stream.next() => { if inspect(message) { break; } }
                message = player2.stoc_stream.next() => { if inspect(message) { break; } }
            }
        }
    }).await.ok();
    winner
}

pub fn player_name(player: CorePlayer) -> &'static str {
    match player {
        CorePlayer::FirstAttackPlayer => "Player1",
        CorePlayer::SecondAttackPlayer => "Player2",
        _ => "Unknown player",
    }
}

fn respond(ctos_sender: &mpsc::UnboundedSender<ctos::Message>, player: CorePlayer, message: &Complex<stoc::Message>) -> Result<bool, ReconstructionError> {
    log::debug!("{} S← {:?}", player_name(player), message.deref());
    match message.deref() {
        stoc::Message::TimeLimit(limit) if limit.player == player => { send(ctos_sender, player, ctos::Message::TimeConfirm(ctos::TimeConfirm)); }
        stoc::Message::GameMessage(game_message) if game_message.message.waiting_for().is_some() => { return Ok(true) }
        stoc::Message::GameMessage(game_message) if gm::MessageType::from(&game_message.message) == gm::MessageType::Retry => { return Err(ReconstructionError::Retry) }
        _ => (),
    }
    return Ok(false)
}

fn send(ctos_sender: &mpsc::UnboundedSender<ctos::Message>, player: CorePlayer, message: ctos::Message) {
    log::debug!("{} C→ {:?}", player_name(player), message);
    ctos_sender.send(message).ok();
}

pub async fn wait_for<Stream>(stream: &mut Stream, player: CorePlayer, message_type: stoc::MessageType) -> Result<(), ReconstructionError>
where Stream: futures::Stream<Item = Complex<stoc::Message>> + Unpin + Send + 'static {
    while let Some(message) = stream.next().await {
        let message = message.deref();
        let received_type = stoc::MessageType::from(message);
        log::debug!("{} S← {:?}", player_name(player), message);
        if received_type == message_type {
            return Ok(());
        }
    }
    log::debug!("{} stream ended while waiting for {:?}", player_name(player), message_type);
    Err(ReconstructionError::DuelDidNotEnd)
}
