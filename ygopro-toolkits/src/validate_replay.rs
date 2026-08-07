//! Replay validation. Drives a replay through either the in-process
//! [`SingleDuel`] engine or an external ygopro-server binary bridged by
//! [`YgoproBinaryProvider`]. Both backends speak
//! `RoomProvider<ctos::Message, Complex<stoc::Message>>`, so the driving logic
//! below is shared and only the room construction differs.

use std::io::Cursor;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use binrw::BinRead;
use futures::SinkExt;
use glob::glob;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio_stream::StreamExt;
use tokio_util::codec::FramedRead;
use tokio_util::codec::FramedWrite;
use tokio_util::codec::LengthDelimitedCodec;

use ygopro::managers::DataManager;
use ygopro::managers::data_manager;
use ygopro_data::complex::Complex;
use ygopro_data::constants::Netplayer;
use ygopro_data::data::Replay;
use ygopro_data::data::ReplayDeck;
use ygopro_data::message::ctos;
use ygopro_data::message::stoc;
use ygopro_handler::RoomProvider;
use ygopro_external_server_bridge::YgoproBinaryFactory;

use crate::common::start_game::*;

#[derive(Debug)]
pub struct ValidationSummary {
    pub response_count: usize,
    pub winner: Option<Netplayer>,
    pub replayed_to_end: bool,
}

pub async fn run(paths: &[PathBuf], wait: Option<u16>, timeout_seconds: u64, server_bin: Option<PathBuf>, server_cwd: Option<PathBuf>) {
    ygopro::init();
    let mut matched_paths = Vec::new();
    for pattern in paths {
        match glob(&pattern.to_string_lossy()) {
            Ok(glob_matches) => matched_paths.extend(glob_matches.filter_map(Result::ok)),
            Err(error) => {
                log::error!("cannot parse pattern {}: {error}", pattern.display());
                std::process::exit(1);
            }
        }
    }
    if matched_paths.is_empty() {
        log::error!("no replay files matched");
        std::process::exit(1);
    }
    let single_file = matched_paths.len() == 1;
    let mut failed_count = 0;
    for path in matched_paths {
        let wait = if single_file { wait } else { None };
        match validate_replay(&path, wait, timeout_seconds, server_bin.clone(), server_cwd.clone()).await {
            Ok(summary) => {
                let winner_text = match summary.winner {
                    Some(winner) => format!("{winner:?}"),
                    None if summary.replayed_to_end => "unknown (surrendered)".to_string(),
                    None => "draw".to_string(),
                };
                log::info!(
                    "{}: replay is valid: {} responses replayed, winner: {winner_text}",
                    path.display(),
                    summary.response_count
                );
            }
            Err(error) => {
                log::error!("{}: replay is invalid: {error}", path.display());
                failed_count += 1;
            }
        }
    }
    if failed_count > 0 {
        std::process::exit(1);
    }
}

pub async fn validate_replay(path: &Path, wait: Option<u16>, timeout_seconds: u64, server_bin: Option<PathBuf>, server_cwd: Option<PathBuf>) -> Result<ValidationSummary, ReconstructionError> {
    let validation_timeout = Duration::from_secs(timeout_seconds);
    let bytes = std::fs::read(path).map_err(ReconstructionError::Io)?;
    let replay = Replay::read_le(&mut Cursor::new(bytes)).map_err(ReconstructionError::Parse)?;
    if replay.is_tag() {
        return Err(ReconstructionError::TagReplayNotSupported);
    }
    if replay.body.datas.is_empty() {
        return Err(ReconstructionError::EmptyReplay);
    }

    let data_manager = data_manager::load();
    let data_manager = data_manager.as_ref().expect("data manager is not initialized");
    check_deck_cards(&replay.body.host_deck, data_manager)?;
    check_deck_cards(&replay.body.client_deck, data_manager)?;

    match (server_bin, server_cwd) {
        (Some(server_bin), Some(server_cwd)) => {
            let factory = YgoproBinaryFactory::new(server_bin.to_string_lossy().into_owned(), server_cwd.to_string_lossy().into_owned());
            let host_info = replay.host_info();
            let seed = replay.header.seed_sequence;
            let mut provider = factory.start("validate".to_string(), host_info, Some(vec![seed])).await.map_err(ReconstructionError::Io)?;
            validate_with_room(&mut provider, replay, wait, validation_timeout).await
        }
        _ => {
            let mut host_info = replay.host_info();
            host_info.time_limit = 0;
            let configuration = ygopro::Configuration {
                no_mask: true,
                no_init_shuffle_deck: true,
                seed_generator: Some(Box::new(move |_duel_count: u8| ygopro_core_wrapper::DuelSeed::Complicated(replay.header.seed_sequence))),
                ..Default::default()
            };
            let (mut host, _duel_task) = ygopro::SingleDuel::new(host_info, configuration);
            validate_with_room(&mut host, replay,  wait, validation_timeout).await
        }
    }
}

async fn validate_with_room<Room: RoomProvider<ctos::Message, Complex<stoc::Message>>>(
    room: &mut Room,
    replay: Replay,
    wait: Option<u16>,
    validation_timeout: Duration,
) -> Result<ValidationSummary, ReconstructionError> {
    let (mut player1, mut player2) = start_duel(&replay, room).await?;
    bridge_viewer(room, wait).await?;
    let response_count = replay.body.datas.len();
    let responses: Vec<ctos::Response> = replay.body.datas.into_iter().map(|data| ctos::Response { response: data.data }).collect();
    let (winner, replayed_to_end) = drive_duel::<Room>(&mut player1, &mut player2, responses, validation_timeout).await?;
    Ok(ValidationSummary { response_count, winner, replayed_to_end })
}

async fn bridge_viewer<Room: RoomProvider<ctos::Message, Complex<stoc::Message>>>(room: &mut Room, wait: Option<u16>) -> Result<(), ReconstructionError> {
    match wait {
        Some(port) => {
            let listener = TcpListener::bind(("0.0.0.0", port)).await.map_err(ReconstructionError::Io)?;
            log::info!("waiting for a viewer to join on port {port}");
            let (socket, viewer_addr) = listener.accept().await.map_err(ReconstructionError::Io)?;
            log::info!("viewer connected: {viewer_addr}");
            bridge_observer(room, socket);
            Ok(())
        }
        None => Ok(()),
    }
}

fn bridge_observer<Room: RoomProvider<ctos::Message, Complex<stoc::Message>>>(room: &mut Room, socket: TcpStream) {
    let (viewer_reader, viewer_writer) = socket.into_split();
    let codec = LengthDelimitedCodec::builder()
        .length_field_length(2)
        .little_endian()
        .new_codec();
    let client_to_server_stream = FramedRead::new(viewer_reader, codec.clone()).filter_map(|result| {
        let frame = result.ok()?;
        let mut cursor = Cursor::new(frame);
        ctos::Message::read_le(&mut cursor).ok()
    });
    let mut server_to_client_stream = room.add(client_to_server_stream);
    tokio::spawn(async move {
        let mut sink = FramedWrite::new(viewer_writer, codec);
        while let Some(message) = server_to_client_stream.next().await {
            let frame = message.data.clone();
            if sink.send(frame).await.is_err() {
                break;
            }
        }
    });
}

fn check_deck_cards(replay_deck: &ReplayDeck, data_manager: &DataManager) -> Result<(), ReconstructionError> {
    let missing_cards: Vec<u32> = replay_deck
        .main
        .iter()
        .chain(replay_deck.extra.iter())
        .filter(|&&code| data_manager.get_card(code).is_none())
        .copied()
        .collect();
    if missing_cards.is_empty() {
        Ok(())
    } else {
        Err(ReconstructionError::MissingCards(missing_cards))
    }
}
