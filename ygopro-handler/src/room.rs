//! # Design
//!
//! [`Room`] is a message hub connecting clients to a game server through a
//! [`RoomProvider`] trait. The provider is a black box that only exposes [`add`](RoomProvider::add).

use std::future::Future;
use std::io::Cursor;

use binrw::BinWrite;
use bytes::Bytes;
use futures::Sink;
use futures::SinkExt;
use futures::Stream;
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

pub trait RoomProvider<ClientToServerMessage, ServerToClientMessage> {
    type ServerToClientStream: Stream<Item = ServerToClientMessage> + Unpin + Send + 'static;
    type FinishFuture: Future<Output = ()> + Unpin + Send + 'static;

    fn add(&mut self, client_to_server_stream: impl Stream<Item = ClientToServerMessage> + Unpin + Send + 'static) -> Self::ServerToClientStream;
    fn get_finish_signal(&mut self) -> Self::FinishFuture;
}

fn create_sender<SinkType, Data>(sink: SinkType) -> mpsc::UnboundedSender<Data>
where
    SinkType: Sink<Data> + Unpin + Send + 'static,
    Data: Send + 'static,
{
    let (sender, mut receiver) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut sink = sink;
        while let Some(message) = receiver.recv().await {
            if sink.send(message).await.is_err() {
                break;
            }
        }
    });
    sender
}

async fn forward<ClientToServerStream, Data>(mut stream: ClientToServerStream, sender: mpsc::UnboundedSender<Data>)
where
    ClientToServerStream: Stream<Item = Data> + Unpin,
    Data: Send + 'static,
{
    while let Some(message) = stream.next().await {
        if sender.send(message).is_err() {
            break;
        }
    }
}

pub struct Room<ProviderType, ClientToServerMessage, ServerToClientMessage>
where
    ProviderType: RoomProvider<ClientToServerMessage, ServerToClientMessage>,
    ClientToServerMessage: Send + 'static,
    ServerToClientMessage: Send + 'static,
{
    pub name: String,
    provider: ProviderType,
    client_to_server_senders: Vec<mpsc::UnboundedSender<ClientToServerMessage>>,
    server_to_client_senders: Vec<mpsc::UnboundedSender<ServerToClientMessage>>,
    server_to_client_bytes_senders: Vec<mpsc::UnboundedSender<Bytes>>,
    pub data: anymap3::Map<dyn std::any::Any + Send + Sync>,
}

impl<ProviderType, ClientToServerMessage, ServerToClientMessage> Room<ProviderType, ClientToServerMessage, ServerToClientMessage>
where
    ProviderType: RoomProvider<ClientToServerMessage, ServerToClientMessage>,
    ClientToServerMessage: Send + 'static,
    ServerToClientMessage: Send + 'static,
{
    pub fn new(provider: ProviderType) -> Self {
        Self {
            name: String::new(),
            provider,
            client_to_server_senders: Vec::new(),
            server_to_client_senders: Vec::new(),
            server_to_client_bytes_senders: Vec::new(),
            data: Default::default(),
        }
    }

    pub fn add_client<ClientToServerStream, ServerToClientSink>(
        &mut self,
        client_to_server_stream: ClientToServerStream,
        server_to_client_sink: ServerToClientSink,
        bytes_sender: mpsc::UnboundedSender<Bytes>,
    )
    where
        ClientToServerStream: Stream<Item = ClientToServerMessage> + Unpin + Send + 'static,
        ServerToClientSink: Sink<ServerToClientMessage> + Unpin + Send + 'static,
    {
        let server_to_client_sender = create_sender(server_to_client_sink);
        let server_to_client_sender_clone = server_to_client_sender.clone();

        let (client_to_server_sender, client_to_server_receiver) = mpsc::unbounded_channel();
        tokio::spawn(forward(client_to_server_stream, client_to_server_sender.clone()));

        let mut server_to_client_stream = self.provider.add(UnboundedReceiverStream::new(client_to_server_receiver));

        tokio::spawn(async move {
            while let Some(message) = server_to_client_stream.next().await {
                server_to_client_sender_clone.send(message).ok();
            }
        });

        self.server_to_client_senders.push(server_to_client_sender);
        self.server_to_client_bytes_senders.push(bytes_sender);
        self.client_to_server_senders.push(client_to_server_sender);
    }

    pub fn send_to_client(&self, index: usize, message: ServerToClientMessage) {
        if let Some(sender) = self.server_to_client_senders.get(index) {
            sender.send(message).ok();
        }
    }

    pub fn send_to_server(&self, index: usize, message: ClientToServerMessage) {
        if let Some(sender) = self.client_to_server_senders.get(index) {
            sender.send(message).ok();
        }
    }

    pub fn broadcast_bytes(&self, message: Bytes) {
        for sender in &self.server_to_client_bytes_senders {
            sender.send(message.clone()).ok();
        }
    }

    pub fn broadcast_message<MessageType>(&self, message: &MessageType)
    where
        MessageType: BinWrite,
        for<'a> <MessageType as BinWrite>::Args<'a>: Default,
    {
        let mut writer = Cursor::new(Vec::new());
        message.write_le(&mut writer).ok();
        let bytes = Bytes::from(writer.into_inner());
        self.broadcast_bytes(bytes);
    }
}
