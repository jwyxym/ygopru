use std::future::Future;
use std::io::Cursor;
use std::ops::Deref;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use binrw::BinRead;
use futures::FutureExt;
use futures::SinkExt;
use futures::StreamExt;
use futures::channel::mpsc;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio_util::codec::FramedRead;
use tokio_util::codec::FramedWrite;
use tokio_util::codec::LengthDelimitedCodec;
use ygopro::Configuration;
use ygopro::SingleDuel;
use ygopro_core_wrapper::DuelSeed;
use ygopro_data::complex::Complex;
use ygopro_data::constants::Color;
use ygopro_data::constants::CorePlayer;
use ygopro_data::data::Replay;
use ygopro_data::data::ReplayData;
use ygopro_data::data::Response;
use ygopro_data::message::HostInfo;
use ygopro_data::message::ctos;
use ygopro_data::message::gm;
use ygopro_data::message::stoc;
use ygopro_external_server_bridge::YgoproBinaryFactory;
use ygopro_external_server_bridge::YgoproBinaryProvider;
use ygopro_handler::RoomProvider;

use crate::common::start_game::*;

pub async fn run(path: &Path, port: u16, server_bin: Option<PathBuf>, server_cwd: Option<PathBuf>) {
    ygopro::init();
    let bytes = std::fs::read(path).expect("cannot read replay file");
    let replay = Replay::read_le(&mut Cursor::new(bytes)).expect("cannot parse replay file");
    let host_info = replay.host_info();
    let (client_to_server_sender1, client_to_server_receiver1) = mpsc::unbounded();
    let (client_to_server_sender2, client_to_server_receiver2) = mpsc::unbounded();
    let (server_to_client_sender1, server_to_client_receiver1) = mpsc::unbounded();
    let (server_to_client_sender2, server_to_client_receiver2) = mpsc::unbounded();
    let (mut streams_sender, streams) = mpsc::unbounded();
    match (server_bin, server_cwd) {
        (Some(binary_path), Some(working_directory)) => {
            let binary_path = binary_path.to_string_lossy().into_owned();
            let working_directory = working_directory.to_string_lossy().into_owned();
            dispatcher(YgoproBinaryFactory::new(binary_path, working_directory), replay, client_to_server_receiver1, client_to_server_receiver2, server_to_client_sender1, server_to_client_sender2);
        }
        _ => {
            dispatcher(SingleDuelFactory, replay, client_to_server_receiver1, client_to_server_receiver2, server_to_client_sender1, server_to_client_sender2);
        }
    }
    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await.expect("failed to bind the port");
    log::info!("tsukuyomi listening on port {port}");
    bridge_loop(streams, host_info, client_to_server_sender1, server_to_client_receiver1, client_to_server_sender2, server_to_client_receiver2);
    loop {
        let (socket, client_addr) = listener.accept().await.expect("failed to accept");
        log::info!("player connected: {client_addr}");
        futures::SinkExt::send(&mut streams_sender, socket).await.ok();
    }
}

pub trait RoomFactory {
    type Room: RoomProvider<ctos::Message, Complex<stoc::Message>>;
    fn create_room(&self, replay: Replay) -> impl Future<Output = Result<(Player<Self::Room>, Player<Self::Room>), ReconstructionError>> + Send;
}

impl RoomFactory for YgoproBinaryFactory {
    type Room = YgoproBinaryProvider;

    fn create_room(&self, replay: Replay) -> impl Future<Output = Result<(Player<Self::Room>, Player<Self::Room>), ReconstructionError>> + Send {
        async move {
            let provider = self.start("tsukuyomi".to_string(), replay.host_info(), Some(vec![replay.header.seed_sequence])).await.map_err(ReconstructionError::Io)?;
            drive_replay(provider, replay).await
        }
    }
}

struct SingleDuelFactory;

impl RoomFactory for SingleDuelFactory {
    type Room = SingleDuel;

    fn create_room(&self, replay: Replay) -> impl Future<Output = Result<(Player<Self::Room>, Player<Self::Room>), ReconstructionError>> + Send {
        async move {
            let seed_sequence = replay.header.seed_sequence;
            let mut configuration = Configuration::default();
            configuration.no_mask = true;
            configuration.enable_plugin(ygopro::plugin::no_init_shuffle_deck::NAME);
            configuration.seed_generator = Some(Box::new(move |_duel_count: u8| DuelSeed::Complicated(seed_sequence)));
            let (duel, _handle) = SingleDuel::new(replay.host_info(), configuration);
            drive_replay(duel, replay).await
        }
    }
}

pub async fn drive_replay<Room>(provider: Room, replay: Replay) -> Result<(Player<Room>, Player<Room>), ReconstructionError>
where Room: RoomProvider<ctos::Message, Complex<stoc::Message>> {
    let mut provider = provider;
    let (mut player1, mut player2) = start_duel(&replay, &mut provider).await?;
    let responses: Vec<ctos::Response> = replay.body.datas.iter().map(|data| ctos::Response { response: data.data.clone() }).collect();
    send_responses(&mut player1, &mut player2, responses).await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    for player in [&mut player1, &mut player2] {
        while player.stoc_stream.next().now_or_never().is_some() {}
    }
    Ok((player1, player2))
}

fn dispatcher<Factory, Stream, Sink>(factory: Factory, mut replay: Replay, source1: Stream, source2: Stream, mut sink1: Sink, mut sink2: Sink)
where Stream: futures::Stream<Item = ctos::Message> + Unpin + Send + 'static,
Sink: futures::Sink<Complex<stoc::Message>> + Send + Unpin + 'static,
Factory: RoomFactory + Send + Sync + 'static
{
    let mut current_replay = replay.clone();
    tokio::spawn(async move {
        let (mut inner_player1, mut inner_player2) = factory.create_room(replay.clone()).await.expect("failed to create room");
        let mut swapped = false;
        let mut client_to_server = futures::stream::select(
            source1.map(|message| (0usize, message)),
            source2.map(|message| (1usize, message)),
        );
        loop {
            tokio::select! {
                ctos_message = client_to_server.next() => {
                    if let Some((player_index, message)) = ctos_message {
                        match &message {
                            ctos::Message::Response(response) => current_replay.body.datas.push(ReplayData { data: response.response.clone() }),
                            ctos::Message::Chat(chat) => if let Ok(command) = Command::try_from(&*chat.msg) {
                                process_command(&command, &factory, &mut sink1, &mut sink2, &mut replay, &mut current_replay, &mut inner_player1, &mut inner_player2, &mut swapped).await;
                                continue;
                            },
                            _ => ()
                        }
                        if player_index == 0 { inner_player1.ctos_sender.send(message).ok(); } 
                        else                 { inner_player2.ctos_sender.send(message).ok(); }
                    }
                }
                stoc_message1 = inner_player1.stoc_stream.next() => {
                    if let Some(message) = stoc_message1 {
                        if stoc::MessageType::from(message.deref()) == stoc::MessageType::DuelEnd {
                            (inner_player1, inner_player2) = operate(&factory, replay.clone(), swapped).await;
                        } else if !intercept_message(&message) {
                            sink1.send(message).await.ok();
                        }
                    }
                }
                stoc_message2 = inner_player2.stoc_stream.next() => {
                    if let Some(message) = stoc_message2 {
                        if stoc::MessageType::from(message.deref()) == stoc::MessageType::DuelEnd {
                            (inner_player1, inner_player2) = operate(&factory, replay.clone(), swapped).await;
                        } else if !intercept_message(&message) {
                            sink2.send(message).await.ok();
                        }
                    }
                }
            }     
        }
    });
}

fn intercept_message(message: &stoc::Message) -> bool {
    if let stoc::Message::GameMessage(m) = message {
        if gm::MessageType::from(&m.message) == gm::MessageType::Win {
            return true;
        }
    }
    match stoc::MessageType::from(message) {
        stoc::MessageType::DuelEnd => return true,
        stoc::MessageType::Replay => return true,
        _ => ()
    }
    false
}

async fn operate<Factory: RoomFactory>(factory: &Factory, replay: Replay, swapped: bool) -> (Player<Factory::Room>, Player<Factory::Room>) {
    let (player1, player2) = factory.create_room(replay).await.expect("failed to create room");
    log::info!("Room created.");
    player1.ctos_sender.send(ctos::RequestField.into()).ok();
    player2.ctos_sender.send(ctos::RequestField.into()).ok();
    if swapped { (player2, player1) } else { (player1, player2) }
}

fn bridge_loop(mut streams: mpsc::UnboundedReceiver<TcpStream>, host_info: HostInfo, sender1: mpsc::UnboundedSender<ctos::Message>, receiver1: mpsc::UnboundedReceiver<Complex<stoc::Message>>, sender2: mpsc::UnboundedSender<ctos::Message>, receiver2: mpsc::UnboundedReceiver<Complex<stoc::Message>>) {
    let mut receiver1_container = Some(receiver1);
    let mut receiver2_container = Some(receiver2);
    let mut bridge1: Option<JoinHandle<mpsc::UnboundedReceiver<Complex<stoc::Message>>>> = None;
    let mut bridge2: Option<JoinHandle<mpsc::UnboundedReceiver<Complex<stoc::Message>>>> = None;
    tokio::spawn(async move {
        loop {
            tokio::select! {
                stream = streams.next() => {
                    let Some(stream) = stream else { break };
                    if let Some(receiver) = receiver1_container.take() {
                        bridge1 = Some(bridge_player(stream, CorePlayer::FirstAttackPlayer, host_info.clone(), sender1.clone(), receiver));
                    } else if let Some(receiver) = receiver2_container.take() {
                        bridge2 = Some(bridge_player(stream, CorePlayer::SecondAttackPlayer, host_info.clone(), sender2.clone(), receiver));
                    } else {
                        log::warn!("rejecting connection: both players are connected");
                        drop(stream);
                    }
                }
                reclaimed1 = async {
                    let Some(handle) = bridge1.as_mut() else { return futures::future::pending().await };
                    let receiver = handle.await.expect("bridge task panicked");
                    bridge1 = None;
                    receiver
                } => {
                    receiver1_container = Some(reclaimed1);
                }
                reclaimed2 = async {
                    let Some(handle) = bridge2.as_mut() else { return futures::future::pending().await };
                    let receiver = handle.await.expect("bridge task panicked");
                    bridge2 = None;
                    receiver
                } => {
                    receiver2_container = Some(reclaimed2);
                }
            }
        }
    });
}

fn bridge_player(socket: TcpStream, player: CorePlayer, host_info: HostInfo, client_to_server_sender: mpsc::UnboundedSender<ctos::Message>, mut server_to_client_receiver: mpsc::UnboundedReceiver<Complex<stoc::Message>>) -> JoinHandle<mpsc::UnboundedReceiver<Complex<stoc::Message>>> {
    let (reader, writer) = socket.into_split();
    let codec = LengthDelimitedCodec::builder().length_field_length(2).little_endian().new_codec();
    let mut client_to_server_stream = FramedRead::new(reader, codec.clone());
    tokio::spawn(async move {
        while let Some(Ok(message)) = client_to_server_stream.next().await {
            if message[0] == u8::from(ctos::MessageType::JoinGame) { break; }
        }
        let mut server_to_client_sink = FramedWrite::new(writer, codec);
        server_to_client_sink.send(Complex::from_message(stoc::Message::JoinGame(stoc::JoinGame { info: host_info.clone() })).data).await.ok();
        server_to_client_sink.send(Complex::from_message(stoc::Message::TypeChange(stoc::TypeChange { player: player.into(), host: player == CorePlayer::FirstAttackPlayer })).data).await.ok();
        client_to_server_sender.unbounded_send(ctos::RequestField.into()).ok();
        loop {
            tokio::select! {
                frame = client_to_server_stream.next() => {
                    let Some(Ok(frame)) = frame else { break };
                    let mut cursor = Cursor::new(frame);
                    let Some(ctos_message) = ctos::Message::read_le(&mut cursor).ok() else { continue };
                    if client_to_server_sender.unbounded_send(ctos_message).is_err() { break; }
                }
                stoc_message = server_to_client_receiver.next() => {
                    let Some(stoc_message) = stoc_message else { break };
                    if server_to_client_sink.send(stoc_message.data).await.is_err() { break; }
                }
            }
        }
        server_to_client_receiver
    })
}

enum Command {
    Save,
    Swap,
    Back,
    Back2,
    Restart,
    Help,
    Clear,
    Exit
}

impl TryFrom<&str> for Command {
    type Error = ();
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "save" => Ok(Command::Save),
            "swap" => Ok(Command::Swap),
            "back" => Ok(Command::Back),
            "back2" => Ok(Command::Back2),
            "restart" => Ok(Command::Restart),
            "help" => Ok(Command::Help),
            "clear" => Ok(Command::Clear),
            "exit" => Ok(Command::Exit),
            _ => Err(()),
        }
    }
}

async fn process_command<Factory: RoomFactory, Sink>(
    command: &Command,
    factory: &Factory,
    sink1: &mut Sink,
    sink2: &mut Sink,
    origin_replay: &mut Replay,
    current_replay: &mut Replay,
    inner_player1: &mut Player<Factory::Room>,
    inner_player2: &mut Player<Factory::Room>,
    swapped: &mut bool,
) where Sink: futures::Sink<Complex<stoc::Message>> + Unpin {
    match command {
        Command::Save => {
            *origin_replay = current_replay.clone();
            let message = Complex::from_message(stoc::Message::Chat(stoc::Chat { player: Color::Green.into(), msg: "[tsukuyomi]: 进度已保存。".into() }));
            sink1.send(message.clone()).await.ok();
            sink2.send(message).await.ok();
        },
        Command::Swap => {
            std::mem::swap(inner_player1, inner_player2);
            *swapped = !*swapped;
            inner_player1.ctos_sender.send(ctos::RequestField.into()).ok();
            inner_player2.ctos_sender.send(ctos::RequestField.into()).ok();
        },
        Command::Back => {
            while matches!(current_replay.body.datas.last(), Some(data) if matches!(&data.data, Response::Cancel) || matches!(&data.data, Response::Unknown(bytes) if bytes.as_slice() == [255, 255, 255, 255])) { current_replay.body.datas.pop(); }
            current_replay.body.datas.pop();
            while matches!(current_replay.body.datas.last(), Some(data) if matches!(&data.data, Response::Cancel) || matches!(&data.data, Response::Unknown(bytes) if bytes.as_slice() == [255, 255, 255, 255])) { current_replay.body.datas.pop(); }
            (*inner_player1, *inner_player2) = operate(factory, current_replay.clone(), *swapped).await;
        },
        Command::Back2 => {
            current_replay.body.datas.pop();
            (*inner_player1, *inner_player2) = operate(factory, current_replay.clone(), *swapped).await;
        },
        Command::Restart => (*inner_player1, *inner_player2) = operate(factory, origin_replay.clone(), *swapped).await,
        Command::Help => {
            let message1 = Complex::from_message(stoc::Message::Chat(stoc::Chat { player: Color::Pink.into(), msg: "[tsukuyomi]: save 保存当前进度 / swap 交换玩家 / back 回到上一个决策 / back2 回退一个响应".into() }));
            let message2 = Complex::from_message(stoc::Message::Chat(stoc::Chat { player: Color::Pink.into(), msg: "[tsukuyomi]: restart 恢复到保存的进度 / clear 重置到决斗开始 / help 帮助 / exit 退出".into() }));
            sink1.send(message1.clone()).await.ok();
            sink1.send(message2.clone()).await.ok();
            sink2.send(message1).await.ok();
            sink2.send(message2).await.ok();
        },
        Command::Clear => {
            current_replay.body.datas.clear();
            (*inner_player1, *inner_player2) = operate(factory, current_replay.clone(), *swapped).await;
        },
        Command::Exit => std::process::exit(0),
    }
}
