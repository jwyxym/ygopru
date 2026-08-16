use std::convert::Infallible;
use std::net::SocketAddr;

use ygopro_data::complex::Complex;
use ygopro_data::constants::CorePlayer;
use ygopro_data::constants::Netplayer;
use ygopro_data::message::ctos;
use ygopro_data::message::stoc;
use ygopro_data::message::gm;

use crate::IntoResponse;
use crate::handler::Bundle;
use crate::handler::FromRequest;

pub struct Request<Message, Extra> {
    pub message: Message,
    pub extra: Extra,
}

macro_rules! impl_extractable {
    ($extra:ty) => {
        impl<Message, State, Res> FromRequest<Request<Message, $extra>, State, Res> for $extra
        where
            Message: Send,
            State: Send,
            Res: Send,
        {
            fn from_request(bundle: &mut Bundle<Request<Message, $extra>, State, Res>) -> Option<Self> {
                Some(bundle.request.extra)
            }
        }
    };
}

impl_extractable!(SocketAddr);
impl_extractable!(Netplayer);
impl_extractable!(CorePlayer);

impl<Message, Extra, State, Res> FromRequest<Request<Message, Extra>, State, Res> for &Message
where
    Message: Send,
    Extra: Send,
    State: Send,
    Res: Send,
{
    fn from_request(bundle: &mut Bundle<Request<Message, Extra>, State, Res>) -> Option<Self> {
        Some(unsafe { &*(&bundle.request.message as *const Message) })
    }
}

macro_rules! impl_variant_ref {
    ($message_mod:ident, $variant:ident) => {
        impl<Extra, State, Res> FromRequest<Request<$message_mod::Message, Extra>, State, Res> for &$message_mod::$variant
        where
            Extra: Send,
            State: Send,
            Res: Send,
        {
            fn from_request(bundle: &mut Bundle<Request<$message_mod::Message, Extra>, State, Res>) -> Option<Self> {
                if let $message_mod::Message::$variant(inner) = &bundle.request.message {
                    Some(unsafe { &*(inner as *const $message_mod::$variant) })
                } else {
                    None
                }
            }
        }

        impl<State, Res> FromRequest<$message_mod::Message, State, Res> for &$message_mod::$variant
        where
            State: Send,
            Res: Send,
        {
            fn from_request(bundle: &mut Bundle<$message_mod::Message, State, Res>) -> Option<Self> {
                if let $message_mod::Message::$variant(inner) = &bundle.request {
                    Some(unsafe { &*(inner as *const $message_mod::$variant) })
                } else {
                    None
                }
            }
        }
    };
}

macro_rules! impl_variant_complex_ref {
    ($message_mod:ident, $variant:ident) => {
        impl<Extra, State, Res> FromRequest<Request<Complex<$message_mod::Message>, Extra>, State, Res> for &$message_mod::$variant
        where
            Extra: Send,
            State: Send,
            Res: Send,
        {
            fn from_request(bundle: &mut Bundle<Request<Complex<$message_mod::Message>, Extra>, State, Res>) -> Option<Self> {
                if let $message_mod::Message::$variant(inner) = &*bundle.request.message {
                    Some(unsafe { &*std::ptr::from_ref(inner) })
                } else {
                    None
                }
            }
        }

        impl<State, Res> FromRequest<Complex<$message_mod::Message>, State, Res> for &$message_mod::$variant
        where
            State: Send,
            Res: Send,
        {
            fn from_request(bundle: &mut Bundle<Complex<$message_mod::Message>, State, Res>) -> Option<Self> {
                if let $message_mod::Message::$variant(inner) = &*bundle.request {
                    Some(unsafe { &*std::ptr::from_ref(inner) })
                } else {
                    None
                }
            }
        }
    };
}

macro_rules! impl_ctos {
    ($($variant:ident = $flag:literal),* $(,)?) => {
        $( impl_variant_ref!(ctos, $variant); )*
        $( impl_variant_complex_ref!(ctos, $variant); )*
        $( impl_variant_response!(ctos, $variant); )*
    };
}

macro_rules! impl_stoc {
    ($($variant:ident = $flag:literal),* $(,)?) => {
        $( impl_variant_ref!(stoc, $variant); )*
        $( impl_variant_complex_ref!(stoc, $variant); )*
        $( impl_variant_response!(stoc, $variant); )*
    };
}

macro_rules! impl_gm {
    ($($variant:ident = $flag:literal),* $(,)?) => {
        $( impl_variant_ref!(gm, $variant); )*
        $( impl_variant_complex_ref!(gm, $variant); )*
        $( impl_variant_response!(gm, $variant); )*
    };
}

pub enum Response<Message> {
    /// Continue processing the message as normal.
    Continue,
    /// Message will be replaced with the given message when sending to its target.
    Replace(Message),
    /// Message will be replaced with multiple messages when sending to its target.
    ReplaceMultiple(Vec<Message>),
    /// This message will not send to its target.
    Swallow,
    /// This message will not send to its target, and stop current room.
    Terminate,
    /// This message will not send to its target, and kick its source.
    Kick,
}

impl<Req, State, Message> FromRequest<Req, State, Response<Message>> for &mut Response<Message> where Req: Send, State: Send, Message: Send + Sync {
    fn from_request(bundle: &mut Bundle<Req, State, Response<Message>>) -> Option<Self> {
        Some(unsafe { &mut *(&mut bundle.response as *mut Response<Message>) })
    }
}

impl<Message> std::ops::Mul for Response<Message> {
    type Output = Response<Message>;

    fn mul(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Response::Continue, other) | (other, Response::Continue) => other,
            (Response::Kick, _) | (_, Response::Kick) => Response::Kick,
            (Response::Terminate, _) | (_, Response::Terminate) => Response::Terminate,
            (Response::Swallow, _) | (_, Response::Swallow) => Response::Swallow,
            (Response::Replace(lhs_message), Response::Replace(rhs_message)) => {
                Response::ReplaceMultiple(vec![lhs_message, rhs_message])
            }
            (Response::Replace(message), Response::ReplaceMultiple(mut messages)) => {
                messages.insert(0, message);
                Response::ReplaceMultiple(messages)
            }
            (Response::ReplaceMultiple(mut messages), Response::Replace(message)) => {
                messages.push(message);
                Response::ReplaceMultiple(messages)
            }
            (Response::ReplaceMultiple(mut lhs_messages), Response::ReplaceMultiple(mut rhs_messages)) => {
                lhs_messages.append(&mut rhs_messages);
                Response::ReplaceMultiple(lhs_messages)
            }
        }
    }
}

impl<Message> Default for Response<Message> {
    fn default() -> Self {
        Response::Continue
    }
}

impl IntoResponse<Response<ctos::Message>> for ctos::Message {
    fn into_response(self) -> Response<ctos::Message> {
        Response::Replace(self)
    }
}

impl IntoResponse<Response<stoc::Message>> for stoc::Message {
    fn into_response(self) -> Response<stoc::Message> {
        Response::Replace(self)
    }
}

impl IntoResponse<Response<gm::Message>> for gm::Message {
    fn into_response(self) -> Response<gm::Message> {
        Response::Replace(self)
    }
}

macro_rules! impl_variant_response {
    ($message_mod:ident, $variant:ident) => {
        impl IntoResponse<Response<$message_mod::Message>> for $message_mod::$variant {
            fn into_response(self) -> Response<$message_mod::Message> {
                Response::Replace($message_mod::Message::$variant(self))
            }
        }
    };
}

impl<Message> IntoResponse<Response<Message>> for () {
    fn into_response(self) -> Response<Message> {
        Response::Continue
    }
}

impl<Message> IntoResponse<Response<Message>> for Infallible {
    fn into_response(self) -> Response<Message> {
        Response::Continue
    }
}

impl<Message> IntoResponse<Response<Message>> for Vec<Message> {
    fn into_response(self) -> Response<Message> {
        Response::ReplaceMultiple(self)
    }
}

impl<Message> IntoResponse<Response<Message>> for bool {
    fn into_response(self) -> Response<Message> {
        if self { Response::Terminate } else { Response::Continue }
    }
}

impl<Message> IntoResponse<Response<Message>> for &'static str {
    fn into_response(self) -> Response<Message> {
        match self {
            "continue" => Response::Continue,
            "terminate" => Response::Terminate,
            "kick" => Response::Kick,
            "cancel" | "_cancel" => Response::Swallow,
            _ => Response::Continue,
        }
    }
}

impl <Message> IntoResponse<Response<Message>> for Option<Message> {
    fn into_response(self) -> Response<Message> {
        match self {
            Some(message) => Response::Replace(message),
            None => Response::Continue,
        }
    }
}

impl<Message, Response1, Response2> IntoResponse<Response<Message>> for Result<Response1, Response2>
where Response1: IntoResponse<Response<Message>>, Response2: IntoResponse<Response<Message>> {
    fn into_response(self) -> Response<Message> {
        match self {
            Ok(response1) => response1.into_response(),
            Err(response2) => response2.into_response(),
        }
    }
}

impl<Req, State, Res> FromRequest<Req, State, Res> for &mut crate::StopFlag
where Req: Send, State: Send, Res: Send {
    fn from_request(bundle: &mut Bundle<Req, State, Res>) -> Option<Self> {
        Some(unsafe { &mut *(&mut bundle.stop_flag as *mut crate::StopFlag) })
    }
}

ygopro_data::every_client_to_server_flat_message!(impl_ctos);
ygopro_data::every_server_to_client_flat_message!(impl_stoc);
ygopro_data::every_game_message_flat_message!(impl_gm);

pub trait ContainsMap {
    fn get_map(&self) -> &anymap3::Map<dyn anymap3::CloneAny + Send>;
}

pub trait ContainsMapMut {
    fn get_map(&mut self) -> &mut anymap3::Map<dyn std::any::Any + Send>;
}
