use std::io::Cursor;
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::LazyLock;

use base64::Engine;
use binrw::BinRead;
use binrw::BinWrite;
use bytes::Bytes;
use futures::SinkExt;
use futures::Stream;
use futures::StreamExt;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::codec::FramedRead;
use tokio_util::codec::FramedWrite;
use tokio_util::codec::LengthDelimitedCodec;

use ygopro_core_wrapper::random::SEED_COUNT;
use ygopro_data::complex::Complex;
use ygopro_data::message::ctos;
use ygopro_data::message::stoc;
use ygopro_data::message::HostInfo;

use ygopro_handler::RoomProvider;

pub struct YgoproBinaryFactory {
    binary_path: String,
    working_directory: String,
}

impl YgoproBinaryFactory {
    pub fn new(binary_path: String, working_directory: String) -> Self {
        Self {
            binary_path,
            working_directory,
        }
    }

    fn generate_args(host_info: &HostInfo, seed: &Option<Vec<[u32; SEED_COUNT]>>) -> Vec<String> {
        let mut args = vec![
            "0".to_string(),
            host_info.lflist.to_string(),
            u8::from(host_info.rule).to_string(),
            (host_info.mode as u8).to_string(),
            (host_info.duel_rule as u8).to_string(),
            if host_info.no_check_deck { "T".to_string() } else { "F".to_string() },
            if host_info.no_shuffle_deck { "T".to_string() } else { "F".to_string() },
            host_info.start_lp.to_string(),
            host_info.start_hand.to_string(),
            host_info.draw_count.to_string(),
            host_info.time_limit.to_string(),
            "0".to_string(),
        ];
        if let Some(seeds) = seed {
            for seed in seeds {
                args.push(encode_seed(seed));
            }
        }
        args
    }

    pub async fn start(&self, name: String, host_info: HostInfo, seed: Option<Vec<[u32; SEED_COUNT]>>) -> std::io::Result<YgoproBinaryProvider> {
        let mut child = Command::new(&self.binary_path)
            .current_dir(&self.working_directory)
            .args(Self::generate_args(&host_info, &seed))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "stdout pipe missing"))?;
        let stderr = child.stderr.take().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "stderr pipe missing"))?;

        let mut stdout_reader = BufReader::new(stdout);
        let mut port_line = String::new();
        stdout_reader.read_line(&mut port_line).await?;
        let port: u16 = port_line.trim().parse().map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let server_address = SocketAddr::new([127, 0, 0, 1].into(), port);

        tokio::spawn(async move {
            let mut stderr_reader = BufReader::new(stderr);
            let mut stdout_line = String::new();
            let mut stderr_line = String::new();
            loop {
                tokio::select! {
                    result = stdout_reader.read_line(&mut stdout_line) => match result {
                        Ok(0) => { log::info!("{name} exited"); break }
                        Ok(_) => { log::info!("[{name}] {}", stdout_line.trim_end()); stdout_line.clear() }
                        Err(_) => { log::info!("{name} exited"); break }
                    },
                    result = stderr_reader.read_line(&mut stderr_line) => match result {
                        Ok(0) => { log::info!("{name} exited"); break }
                        Ok(_) => { log::error!("[{name}] {}", stderr_line.trim_end()); stderr_line.clear() }
                        Err(_) => { log::info!("{name} exited"); break }
                    },
                }
            }
        });

        Ok(YgoproBinaryProvider { server_address })
    }
}

pub struct YgoproBinaryProvider {
    server_address: SocketAddr,
}

impl YgoproBinaryProvider {
    fn spawn_proxy<ServerToClientMessage>(&self, client_to_server_stream: impl Stream<Item = ctos::Message> + Unpin + Send + 'static, sender: mpsc::UnboundedSender<ServerToClientMessage>) 
    where ServerToClientMessage: From<stoc::Message> + Send + 'static {
        let server_address = self.server_address;

        tokio::spawn(async move {
            let connection = match TcpStream::connect(server_address).await {
                Ok(connection) => connection,
                Err(error) => {
                    log::error!("Failed to connect to ygopro at {}: {:?}", server_address, error);
                    return;
                }
            };

            let (reader, writer) = connection.into_split();
            let mut framed_reader = FramedRead::new(reader, FRAME_CODEC.clone());
            let mut framed_writer = FramedWrite::new(writer, FRAME_CODEC.clone());
            let mut client_to_server_stream = client_to_server_stream;

            loop {
                tokio::select! {
                    frame = framed_reader.next() => match frame {
                        Some(Ok(frame)) => {
                            let message = match decode_stoc_frame(&frame) { Some(message) => message, None => break };
                            if sender.send(message.into()).is_err() { break; }
                        }
                        _ => break,
                    },
                    message = client_to_server_stream.next() => {
                        let message = match message { Some(message) => message, None => break };
                        let payload = match encode_ctos_message(&message) { Some(payload) => payload, None => break };
                        if framed_writer.send(Bytes::from(payload)).await.is_err() {
                            log::warn!("Failed to send CTOS message to ygopro");
                        }
                    }
                }
            }
            log::info!("stoc listener stopped");
        });
    }
}

impl RoomProvider<ctos::Message, stoc::Message> for YgoproBinaryProvider {
    type ServerToClientStream = UnboundedReceiverStream<stoc::Message>;

    fn add(&mut self, client_to_server_stream: impl Stream<Item = ctos::Message> + Unpin + Send + 'static) -> Self::ServerToClientStream {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.spawn_proxy(client_to_server_stream, sender);
        UnboundedReceiverStream::new(receiver)
    }
}

impl RoomProvider<ctos::Message, Complex<stoc::Message>> for YgoproBinaryProvider {
    type ServerToClientStream = UnboundedReceiverStream<Complex<stoc::Message>>;

    fn add(&mut self, client_to_server_stream: impl Stream<Item = ctos::Message> + Unpin + Send + 'static) -> Self::ServerToClientStream {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.spawn_proxy(client_to_server_stream, sender);
        UnboundedReceiverStream::new(receiver)
    }
}

fn encode_ctos_message(message: &ctos::Message) -> Option<Vec<u8>> {
    let mut payload = Vec::new();
    message.write_le(&mut Cursor::new(&mut payload)).ok()?;
    Some(payload)
}

fn decode_stoc_frame(frame: &[u8]) -> Option<stoc::Message> {
    match stoc::Message::read_le(&mut Cursor::new(frame)) {
        Ok(message) => Some(message),
        Err(error) => {
            log::error!("Failed to parse STOC message: {:?}", error);
            None
        }
    }
}

fn encode_seed(seed: &[u32; SEED_COUNT]) -> String {
    let mut bytes = Vec::with_capacity(SEED_COUNT * 4);
    for value in seed {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

static FRAME_CODEC: LazyLock<LengthDelimitedCodec> = LazyLock::new(|| {
    LengthDelimitedCodec::builder()
        .little_endian()
        .length_field_type::<u16>()
        .new_codec()
});
