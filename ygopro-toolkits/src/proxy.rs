use std::fmt::{Debug, Display};
use std::io::Cursor;
use std::net::SocketAddr;

use binrw::BinRead;
use futures::FutureExt;
use futures::SinkExt;
use futures::StreamExt;
use log::{trace, info};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::FramedRead;
use tokio_util::codec::FramedWrite;
use tokio_util::codec::LengthDelimitedCodec;
use ygopro_data::message::{client_to_server, server_to_client};

pub async fn run(target: SocketAddr, port: u32) {
    let server_addr: SocketAddr = format!("0.0.0.0:{port}")
        .parse()
        .expect("Cannot parse the listening socket.");
    let client_listener = TcpListener::bind(server_addr).await.expect("Failed to bind the port");
    loop {
        let (client_socket, client_addr) = client_listener.accept().await.expect("Cannot get listen socket");
        let server_socket = TcpStream::connect(target).await.expect("Cannot get send socket");
        let (client_reader, client_writer) = client_socket.into_split();
        let (server_reader, server_writer) = server_socket.into_split();
        info!("{:} <-> {:} ", client_addr, target);
        run_stream::<client_to_server::Message>(client_reader, server_writer, "C→");
        run_stream::<server_to_client::Message>(server_reader, client_writer, "←S");
    }
}

fn run_stream<M: BinRead + Debug + Send + 'static>(
    reader: impl AsyncRead + Unpin + Send + 'static,
    writer: impl AsyncWrite + Unpin + Send + 'static,
    direction: &'static str,
) where for<'a> <M as BinRead>::Args<'a>: Default {
    tokio::spawn(async move {
        let mut stream = FramedRead::new(reader, LengthDelimitedCodec::builder().length_field_length(2).little_endian().new_codec());
        let mut sink = FramedWrite::new(writer, LengthDelimitedCodec::builder().length_field_length(2).little_endian().new_codec());
        while let Some(Ok(first)) = stream.next().await {
            let mut messages = vec![first];
            loop {
                match stream.next().now_or_never() {
                    Some(Some(Ok(data))) => messages.push(data),
                    _ => break,
                }
            }
            let len = messages.len();
            for (id, data) in messages.into_iter().enumerate() {
                let mut cursor = Cursor::new(&data[..]);
                if let Ok(message_enum) = M::read_le(&mut cursor) {
                    log_message(direction, message_enum, &data[..], id + 1, len);
                }
                sink.send(data.freeze()).await.ok();
            }
        }
        info!("Link dropped.");
    });
}

fn log_message<T: Debug>(leader: &str, message: T, bytes: &[u8], id: impl Display, len: impl Display) {
    info!("[{:}]({:2}/{:2}) {:?}", leader, id, len, message);
    trace!("[{:}]({:2}/{:2}) {:?}", leader, id, len, bytes)
}
