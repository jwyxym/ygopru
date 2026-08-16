use std::ffi::CStr;
use std::ffi::c_char;
use std::io;
use std::io::Cursor;
use std::ops::Deref;
use std::sync::OnceLock;
use std::thread::JoinHandle;
use std::time::Duration;

use base64::Engine;
use binrw::BinRead;
use futures::SinkExt;
use hashbrown::HashMap;
use parking_lot::Mutex;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_stream::StreamExt;
use tokio_util::codec::LengthDelimitedCodec;


#[cfg(feature = "ygomobile_support")]
use jni::JNIEnv;
#[cfg(feature = "ygomobile_support")]
use jni::objects::JClass;
#[cfg(feature = "ygomobile_support")]
use jni::objects::JString;
#[cfg(feature = "ygomobile_support")]
use jni::sys::jint;

use ygopro_core_wrapper::{DuelSeed, random::SEED_COUNT};
use ygopro_data::constants::MasterRule;
use ygopro_data::constants::Mode;
use ygopro_data::constants::Rule;
use ygopro_data::data::ReplayMode;
use ygopro_data::message::HostInfo;
use ygopro_data::message::ctos;
use ygopro_handler::RoomProvider;

use ygopro::managers;
use ygopro::ypk;
use ygopro::init_core;
use ygopro::single_duel::SingleDuelHost;

const START_ALREADY_RUNNING: i32 = -1;
const START_BAD_ARGUMENTS: i32 = -2;
const START_RUNTIME_ERROR: i32 = -3;
const START_BIND_ERROR: i32 = -4;
const START_THREAD_ERROR: i32 = -5;

static SERVER_CONTROL: OnceLock<Mutex<Option<ServerControl>>> = OnceLock::new();
static SERVER_SEEDS: OnceLock<Mutex<Vec<[u32; SEED_COUNT]>>> =
    OnceLock::new();

struct ServerControl {
    shutdown_sender: oneshot::Sender<()>,
    server_thread: JoinHandle<()>,
}

struct ServerArguments {
    port: u16,
    host_info: HostInfo,
    replay_mode: ReplayMode,
    seeds: Vec<[u32; SEED_COUNT]>,
    base_path: String
}

#[unsafe(no_mangle)]
pub extern "C" fn start_server(arguments_pointer: *const c_char) -> i32 {
    let argument_text = read_argument_text(arguments_pointer);
    let arguments = split_command_line(&argument_text);
    start_server_from_arguments(&arguments)
}

pub fn start_server_from_arguments(arguments: &[String]) -> i32 {
    let server_arguments = match parse_server_arguments(arguments) {
        Ok(server_arguments) => server_arguments,
        Err(error_code) => return error_code,
    };

    let server_control_lock = SERVER_CONTROL.get_or_init(|| Mutex::new(None));
    let mut server_control = server_control_lock.lock();
    if server_control.is_some() {
        return START_ALREADY_RUNNING;
    }

    *SERVER_SEEDS.get_or_init(|| Mutex::new(Vec::new())).lock() = server_arguments.seeds.clone();

    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let (start_result_sender, start_result_receiver) = std::sync::mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                log::error!("Failed to create Tokio runtime: {error}");
                start_result_sender.send(Err(START_RUNTIME_ERROR)).ok();
                return;
            }
        };

        runtime.block_on(async move {
            init(&server_arguments);
            if let Err(error) =
                run_tcp_server(server_arguments, shutdown_receiver, start_result_sender).await
            {
                log::error!("TCP server stopped with error: {error}");
            }
        });
        runtime.shutdown_timeout(Duration::from_secs(2));
    });

    let port = match start_result_receiver.recv() {
        Ok(Ok(port)) => port,
        Ok(Err(error_code)) => {
            server_thread.join().ok();
            return error_code;
        }
        Err(_) => {
            server_thread.join().ok();
            return START_THREAD_ERROR;
        }
    };

    *server_control = Some(ServerControl {
        shutdown_sender,
        server_thread,
    });
    port as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn stop_server() {
    let server_control = {
        let server_control_lock = SERVER_CONTROL.get_or_init(|| Mutex::new(None));
        server_control_lock.lock().take()
    };

    if let Some(server_control) = server_control {
        server_control.shutdown_sender.send(()).ok();
        server_control.server_thread.join().ok();
    }
}

#[cfg(feature = "ygomobile_support")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_garymb_ygomobile_network_YGOServer_startServer(mut environment: JNIEnv, _class: JClass, arguments: JString) -> jint {
    let argument_text = match environment.get_string(&arguments) {
        Ok(argument_text) => argument_text.to_string_lossy().to_string(),
        Err(_) => return START_BAD_ARGUMENTS as jint,
    };
    let arguments = split_command_line(&argument_text);
    start_server_from_arguments(&arguments) as jint
}

#[cfg(feature = "ygomobile_support")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_garymb_ygomobile_network_YGOServer_stopServer(_environment: JNIEnv, _class: JClass) {
    stop_server();
}

async fn run_tcp_server(server_arguments: ServerArguments, mut shutdown_receiver: oneshot::Receiver<()>, start_result_sender: std::sync::mpsc::Sender<Result<u16, i32>>) -> io::Result<()> {
    let listener = match TcpListener::bind(format!("0.0.0.0:{}", server_arguments.port)).await {
        Ok(listener) => listener,
        Err(error) => {
            start_result_sender.send(Err(START_BIND_ERROR)).ok();
            return Err(error);
        }
    };
    let port = match listener.local_addr() {
        Ok(address) => address.port(),
        Err(error) => {
            start_result_sender.send(Err(START_BIND_ERROR)).ok();
            return Err(error);
        }
    };
    start_result_sender.send(Ok(port)).ok();
    log::info!("Listening on port {port}");

    let mut configuration = ygopro::Configuration::default();
	configuration.seed_generator = Some(Box::new(seed_generator));
    configuration.enable_plugin_with_configuration(ygopro::plugin::replay::NAME, ygopro::plugin::replay::Configuration { mode: server_arguments.replay_mode });
    let (mut duel, duel_handle) = SingleDuelHost::new(server_arguments.host_info, configuration);
    let mut client_tasks = Vec::new();

    loop {
        tokio::select! {
            _ = &mut shutdown_receiver => {
                break;
            }
            accepted = listener.accept() => {
                let (stream, _address) = accepted?;
                let (reader, writer) = stream.into_split();
                let framed_read = LengthDelimitedCodec::builder()
                    .length_field_type::<u16>()
                    .little_endian()
                    .new_read(reader);
                let mut framed_write = LengthDelimitedCodec::builder()
                    .length_field_type::<u16>()
                    .little_endian()
                    .new_write(writer);

                let client_to_server_stream = framed_read.filter_map(|result| match result {
                    Ok(frame) => {
                        let mut cursor = Cursor::new(&frame);
                        ctos::Message::read_le(&mut cursor).ok().inspect(|message| {
                            log::trace!("CTOS: {message:?}");
                        })
                    }
                    Err(_) => None,
                });

                let mut server_to_client_stream = duel.add(client_to_server_stream);

                client_tasks.retain(|task: &tokio::task::JoinHandle<()>| !task.is_finished());
                let client_task = tokio::spawn(async move {
                    while let Some(message) = server_to_client_stream.next().await {
                        log::trace!("STOC: {:?}", message.deref());
                        framed_write.send(message.data).await.ok();
                    }
                });
                client_tasks.push(client_task);
            }
        }
    }

    duel_handle.abort();
    let _ = duel_handle.await;
    for client_task in client_tasks {
        client_task.abort();
        let _ = client_task.await;
    }
    Ok(())
}

fn read_argument_text(arguments_pointer: *const c_char) -> String {
    if arguments_pointer.is_null() {
        return String::new();
    }

    unsafe { CStr::from_ptr(arguments_pointer) }
        .to_string_lossy()
        .to_string()
}

fn split_command_line(arguments: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_argument = String::new();
    let mut in_quotes = false;

    for character in arguments.chars() {
        if character == '"' {
            in_quotes = !in_quotes;
        } else if character.is_whitespace() && !in_quotes {
            if !current_argument.is_empty() {
                result.push(std::mem::take(&mut current_argument));
            }
        } else {
            current_argument.push(character);
        }
    }

    if !current_argument.is_empty() {
        result.push(current_argument);
    }

    result
}

fn parse_server_arguments(arguments: &[String]) -> Result<ServerArguments, i32> {
    if arguments.is_empty() {
        return Ok(ServerArguments {
            port: 0,
            host_info: HostInfo::default(),
            replay_mode: ReplayMode::empty(),
            seeds: Vec::new(),
            base_path: String::from("./")
        });
    }
    if arguments.len() == 1 {
        let port = arguments[0].parse().map_err(|_| START_BAD_ARGUMENTS)?;
        return Ok(ServerArguments {
            port,
            host_info: HostInfo::default(),
            replay_mode: ReplayMode::empty(),
            seeds: Vec::new(),
            base_path: String::from("./")
        });
    }
    if arguments.len() < 12 {
        return Err(START_BAD_ARGUMENTS);
    }

    let port = arguments[0].parse().map_err(|_| START_BAD_ARGUMENTS)?;
    let duel_rule = parse_duel_rule(&arguments[4]);
    let deck_manager = ygopro::managers::deck_manager::load();
    let host_info = HostInfo {
        lflist: deck_manager.as_ref().and_then(|dm| dm.get_lflist_by_index(arguments[1].parse().unwrap_or(0))).map(|l| l.hash).unwrap_or(0),
        rule: Rule::try_from(arguments[2].parse::<u8>().unwrap_or(0)).unwrap_or(Rule::All),
        mode: parse_mode(&arguments[3]),
        duel_rule,
        no_check_deck: arguments[5] == "T",
        no_shuffle_deck: arguments[6] == "T",
        start_lp: arguments[7].parse().unwrap_or(8000),
        start_hand: arguments[8].parse().unwrap_or(5),
        draw_count: arguments[9].parse().unwrap_or(1),
        time_limit: arguments[10].parse().unwrap_or(180),
    };
    let replay_mode: ReplayMode = ReplayMode::from_bits_retain(arguments[11].parse::<u32>().unwrap_or(0));
    let base_path = arguments.get(12).cloned().unwrap_or_else(|| String::from("./"));
    let seeds = arguments
        .iter()
        .skip(15)
        .map(|seed_argument| decode_seed(seed_argument))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ServerArguments {
        port,
        host_info,
        replay_mode,
        seeds,
        base_path
    })
}

fn init(server_arguments: &ServerArguments) {
    configure_resource_paths(&server_arguments);
    ypk::archive_manager::init();
    managers::data_manager::init();
    managers::deck_manager::init();
    init_core();
}

fn configure_resource_paths(server_arguments: &ServerArguments) {
    let mut entries: HashMap<String, String> = HashMap::new();
    entries.insert("path".to_string(), server_arguments.base_path.clone());
	let config = managers::config_manager::ConfigManager::from(entries);
    managers::config_manager::set_global(config);
}

fn parse_mode(argument: &str) -> Mode {
    match argument.parse::<u8>().unwrap_or(0) {
        mode_value if mode_value > 2 => Mode::Single,
        mode_value => Mode::try_from(mode_value).unwrap_or(Mode::Single),
    }
}

fn parse_duel_rule(argument: &str) -> MasterRule {
    if argument == "T" {
        MasterRule::MasterRuleNew
    } else if argument == "F" {
        MasterRule::MasterRule2020
    } else if let Ok(rule_value) = argument.parse::<u8>() {
        if rule_value != 0 {
            MasterRule::try_from(rule_value).unwrap_or(MasterRule::MasterRule2020)
        } else {
            MasterRule::MasterRule2020
        }
    } else {
        MasterRule::MasterRule2020
    }
}

fn decode_seed(seed_argument: &str) -> Result<[u32; SEED_COUNT], i32> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(seed_argument)
        .map_err(|_| START_BAD_ARGUMENTS)?;
    let mut seed = [0u32; SEED_COUNT];
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        if index >= seed.len() {
            break;
        }
        seed[index] = u32::from_le_bytes(chunk.try_into().map_err(|_| START_BAD_ARGUMENTS)?);
    }
    Ok(seed)
}

fn seed_generator(duel_count: u8) -> DuelSeed {
    let seed = SERVER_SEEDS
        .get()
        .and_then(|seeds| seeds.lock().get(duel_count as usize).copied());
    match seed {
        Some(seed) => DuelSeed::Complicated(seed),
        None => DuelSeed::None,
    }
}
