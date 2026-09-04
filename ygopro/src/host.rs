/// Wrapper of Duel instances. 
/// 
/// [`DuelHost`] is a wrapper of [`Duel`] instances.

use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;

use futures::Stream;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;

use ygopro_data::complex::Complex;
use ygopro_data::constants::Mode;
use ygopro_data::constants::Netplayer;
use ygopro_data::message::HostInfo;
use ygopro_data::message::ctos;
use ygopro_data::message::stoc;
use ygopro_data::string::FixedLengthString;
use ygopro_handler::RoomProvider;

use crate::configuration::Configuration;
use crate::duel::Request;
use crate::duel::SendTarget;
use crate::single_duel::SingleDuel;
use crate::tag_duel::TagDuel;

pub struct DuelHost {
    pub(crate) ctos_sender: mpsc::UnboundedSender<Request>,
    pub finished_sender: watch::Sender<bool>,
}

impl DuelHost {
    pub fn new(host_info: HostInfo, configuration: Configuration) -> Self {
        let (request_sender, handle) = if host_info.mode == Mode::Tag {
            let tag_duel = TagDuel::new(host_info.clone(), configuration);
            let request_sender = tag_duel.request_sender.clone();
            let handle = tag_duel.run().expect("duel already started");
            (request_sender, handle)
        } else {
            let single_duel = SingleDuel::new(host_info.clone(), configuration);
            let request_sender = single_duel.request_sender.clone();
            let handle = single_duel.run().expect("duel already started");
            (request_sender, handle)
        };
        let (finished_sender, _) = watch::channel(false);
        let finished_sender_for_host = finished_sender.clone();
        tokio::spawn(async move {
            let _ = handle.await;
            finished_sender.send(true).ok();
        });
        request_sender.send(Request::Message(crate::ygopro_handlers::Request { message: ctos::Message::CreateGame(ctos::CreateGame { host_info, name: FixedLengthString::allocate(), pass: FixedLengthString::allocate() }), extra: Netplayer::Unknown })).ok();
        Self { ctos_sender: request_sender, finished_sender: finished_sender_for_host }
    }

    fn bridge<Item, Convert>(&self, client_to_server_stream: impl Stream<Item = Item> + Unpin + Send + 'static, mut convert: Convert) -> UnboundedReceiverStream<Complex<stoc::Message>> 
    where Item: Send + 'static, 
          Convert: FnMut(Item) -> Option<ctos::Message> + Send + 'static 
    {
        let ctos_sender = self.ctos_sender.clone();
        let (stoc_sender, stoc_receiver) = mpsc::unbounded_channel();
        let (return_sender, return_receiver) = mpsc::unbounded_channel();
        let (position_sender, position_receiver) = tokio::sync::oneshot::channel();
        ctos_sender.send(Request::MessageEx(crate::ygopro_handlers::RequestEx { message: crate::message::ClientJoin { stoc_sender, position_sender: Some(position_sender) }.into(), extra: SendTarget::None })).ok();

        tokio::spawn(async move {
            let mut ctos_stream = Box::pin(client_to_server_stream);
            let mut stoc_stream = UnboundedReceiverStream::new(stoc_receiver);
            let mut my_position = position_receiver.await.unwrap_or(Netplayer::Unknown);
            loop {
                tokio::select! {
                    message = ctos_stream.next() => {
                        match message {
                            Some(item) => if let Some(message) = convert(item) {
                                log::debug!("[←C {my_position:?}] {message:?}");
                                ctos_sender.send(Request::Message(crate::ygopro_handlers::Request { message, extra: my_position })).ok();
                            },
                            None => {
                                ctos_sender.send(Request::Message(crate::ygopro_handlers::Request { message: ctos::Message::LeaveGame(ctos::LeaveGame), extra: my_position })).ok();
                                break;
                            }
                        }
                    }
                    message = stoc_stream.next() => {
                        if let Some(message) = message {
                            match message.deref() {
                                stoc::Message::TypeChange(type_change) => my_position = type_change.player,
                                stoc::Message::LeaveGame(leave_game) => if leave_game.pos == my_position { break },
                                _ => ()
                            };
                            log::debug!("[S→ {my_position:?}] {:?}", message.deref());
                            return_sender.send(message).ok();
                        } else { break; }
                    }
                }
            }
        });
        UnboundedReceiverStream::new(return_receiver)
    }

    fn finish_signal(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let mut finished_receiver = self.finished_sender.subscribe();
        Box::pin(async move {
            let _ = finished_receiver.wait_for(|finished| *finished).await;
        })
    }
}

impl RoomProvider<ctos::Message, Complex<stoc::Message>> for DuelHost {
    type ServerToClientStream = UnboundedReceiverStream<Complex<stoc::Message>>;
    type FinishFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

    fn add(&mut self, client_to_server_stream: impl Stream<Item = ctos::Message> + Unpin + Send + 'static) -> Self::ServerToClientStream {
        self.bridge(client_to_server_stream, |message| Some(message))
    }

    fn get_finish_signal(&mut self) -> Self::FinishFuture {
        self.finish_signal()
    }
}

impl RoomProvider<Complex<ctos::Message>, Complex<stoc::Message>> for DuelHost {
    type ServerToClientStream = UnboundedReceiverStream<Complex<stoc::Message>>;
    type FinishFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

    fn add(&mut self, client_to_server_stream: impl Stream<Item = Complex<ctos::Message>> + Unpin + Send + 'static) -> Self::ServerToClientStream {
        self.bridge(client_to_server_stream, |complex| complex.into_inner())
    }

    fn get_finish_signal(&mut self) -> Self::FinishFuture {
        self.finish_signal()
    }
}
