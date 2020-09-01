use core::pin::Pin;
use core::task::{Context, Poll};

use std::io;

pub mod prelude {
    pub use crate::{Message, ProtocolError};
    pub use actix_send::prelude::{
        actor as ws, async_trait, handler, handler_v2 as ws_handler, ActixSendError, Actor,
        Handler, MapResult,
    };
}

pub use actix_http::ws::{CloseCode, CloseReason, Frame, HandshakeError, Message, ProtocolError};

use actix_codec::{Decoder, Encoder};
use actix_http::ws::{hash_key, Codec};
use actix_send::prelude::{ActixSendError, Actor, Address, Handler, MapResult};
use actix_web::web::{Bytes, BytesMut};
use actix_web::{
    dev::HttpResponseBuilder,
    error::{Error, PayloadError},
    http::{header, Method, StatusCode},
    HttpRequest, HttpResponse,
};
use futures_channel::mpsc::UnboundedReceiver;
use futures_util::stream::Stream;

// start the websocket connection with a given actor address.
pub async fn start<A, S>(
    addr: Address<A>,
    req: &HttpRequest,
    stream: S,
) -> Result<HttpResponse, Error>
where
    A: Actor + Handler + 'static,
    A::Message: From<Result<Message, ProtocolError>>,
    S: Stream<Item = Result<Bytes, PayloadError>> + Unpin + 'static,
    Result<Message, ProtocolError>: MapResult<A::Result, Output = Option<Vec<Message>>>,
{
    let mut res = handshake(&req)?;

    let codec = Codec::new();

    let decode_stream = DecodeStream {
        stream,
        codec,
        buf: BytesMut::new(),
        closed: false,
    };

    let actor_stream = addr.send_stream::<_, _, Result<Message, ProtocolError>>(decode_stream);

    let encode_stream = EncodeStream {
        rx: None,
        stream: actor_stream,
        buf: BytesMut::new(),
        codec,
        queue: Vec::new(),
    };

    Ok(res.streaming(encode_stream))
}

// start the websocket with an extra channel sender.
// the sender can be used to add websocket message to EncodeStream from other threads.
pub async fn start_with_tx<A, S>(
    addr: Address<A>,
    req: &HttpRequest,
    stream: S,
) -> Result<
    (
        HttpResponse,
        futures_channel::mpsc::UnboundedSender<Message>,
    ),
    Error,
>
where
    A: Actor + Handler + 'static,
    A::Message: From<Result<Message, ProtocolError>>,
    S: Stream<Item = Result<Bytes, PayloadError>> + Unpin + 'static,
    Result<Message, ProtocolError>: MapResult<A::Result, Output = Option<Vec<Message>>>,
{
    let mut res = handshake(&req)?;

    let codec = Codec::new();

    let decode_stream = DecodeStream {
        stream,
        codec,
        buf: BytesMut::new(),
        closed: false,
    };

    let actor_stream = addr.send_stream::<_, _, Result<Message, ProtocolError>>(decode_stream);

    let (tx, rx) = futures_channel::mpsc::unbounded::<Message>();

    let encode_stream = EncodeStream {
        rx: Some(rx),
        stream: actor_stream,
        buf: BytesMut::new(),
        codec,
        queue: Vec::new(),
    };

    Ok((res.streaming(encode_stream), tx))
}

/// Prepare `WebSocket` handshake response.
///
/// This function returns handshake `HttpResponse`, ready to send to peer.
/// It does not perform any IO.
pub fn handshake(req: &HttpRequest) -> Result<HttpResponseBuilder, HandshakeError> {
    handshake_with_protocols(req, &[])
}

/// Prepare `WebSocket` handshake response.
///
/// This function returns handshake `HttpResponse`, ready to send to peer.
/// It does not perform any IO.
///
/// `protocols` is a sequence of known protocols. On successful handshake,
/// the returned response headers contain the first protocol in this list
/// which the server also knows.
pub fn handshake_with_protocols(
    req: &HttpRequest,
    protocols: &[&str],
) -> Result<HttpResponseBuilder, HandshakeError> {
    // WebSocket accepts only GET
    if *req.method() != Method::GET {
        return Err(HandshakeError::GetMethodRequired);
    }

    // Check for "UPGRADE" to websocket header
    let has_hdr = if let Some(hdr) = req.headers().get(header::UPGRADE) {
        if let Ok(s) = hdr.to_str() {
            s.to_ascii_lowercase().contains("websocket")
        } else {
            false
        }
    } else {
        false
    };
    if !has_hdr {
        return Err(HandshakeError::NoWebsocketUpgrade);
    }

    // Upgrade connection
    if !req.head().upgrade() {
        return Err(HandshakeError::NoConnectionUpgrade);
    }

    // check supported version
    if !req.headers().contains_key(header::SEC_WEBSOCKET_VERSION) {
        return Err(HandshakeError::NoVersionHeader);
    }
    let supported_ver = {
        if let Some(hdr) = req.headers().get(header::SEC_WEBSOCKET_VERSION) {
            hdr == "13" || hdr == "8" || hdr == "7"
        } else {
            false
        }
    };
    if !supported_ver {
        return Err(HandshakeError::UnsupportedVersion);
    }

    // check client handshake for validity
    if !req.headers().contains_key(header::SEC_WEBSOCKET_KEY) {
        return Err(HandshakeError::BadWebsocketKey);
    }
    let key = {
        let key = req.headers().get(header::SEC_WEBSOCKET_KEY).unwrap();
        hash_key(key.as_ref())
    };

    // check requested protocols
    let protocol = req
        .headers()
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|req_protocols| {
            let req_protocols = req_protocols.to_str().ok()?;
            req_protocols
                .split(',')
                .map(|req_p| req_p.trim())
                .find(|req_p| protocols.iter().any(|p| p == req_p))
        });

    let mut response = HttpResponse::build(StatusCode::SWITCHING_PROTOCOLS)
        .upgrade("websocket")
        .header(header::TRANSFER_ENCODING, "chunked")
        .header(header::SEC_WEBSOCKET_ACCEPT, key.as_str())
        .take();

    if let Some(protocol) = protocol {
        response.header(header::SEC_WEBSOCKET_PROTOCOL, protocol);
    }

    Ok(response)
}

// decode incoming stream. eg: actix_web::web::Payload to a websocket Message.
#[pin_project::pin_project]
struct DecodeStream<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    #[pin]
    stream: S,
    codec: Codec,
    buf: BytesMut,
    closed: bool,
}

impl<S> Stream for DecodeStream<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    // Decode error is packed with websocket Message and return as a Result.
    type Item = Result<Message, ProtocolError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();

        if !*this.closed {
            loop {
                this = self.as_mut().project();
                match Pin::new(&mut this.stream).poll_next(cx) {
                    Poll::Ready(Some(Ok(chunk))) => {
                        this.buf.extend_from_slice(&chunk[..]);
                    }
                    Poll::Ready(None) => {
                        *this.closed = true;
                        break;
                    }
                    Poll::Pending => break,
                    Poll::Ready(Some(Err(e))) => {
                        return Poll::Ready(Some(Err(ProtocolError::Io(io::Error::new(
                            io::ErrorKind::Other,
                            format!("{}", e),
                        )))));
                    }
                }
            }
        }

        match this.codec.decode(this.buf)? {
            None => {
                if *this.closed {
                    Poll::Ready(None)
                } else {
                    Poll::Pending
                }
            }
            Some(frm) => {
                let msg = match frm {
                    Frame::Text(data) => Message::Text(
                        std::str::from_utf8(&data)
                            .map_err(|e| {
                                ProtocolError::Io(io::Error::new(
                                    io::ErrorKind::Other,
                                    format!("{}", e),
                                ))
                            })?
                            .to_string(),
                    ),
                    Frame::Binary(data) => Message::Binary(data),
                    Frame::Ping(s) => Message::Ping(s),
                    Frame::Pong(s) => Message::Pong(s),
                    Frame::Close(reason) => Message::Close(reason),
                    Frame::Continuation(item) => Message::Continuation(item),
                };
                Poll::Ready(Some(Ok(msg)))
            }
        }
    }
}

// encode a stream of websocket message to Bytes.
#[pin_project::pin_project]
struct EncodeStream<S>
where
    // messages come as an option.
    S: Stream<Item = Result<Option<Vec<Message>>, ActixSendError>>,
{
    rx: Option<UnboundedReceiver<Message>>,
    #[pin]
    stream: S,
    codec: Codec,
    buf: BytesMut,
    queue: Vec<Message>,
}

impl<S> Stream for EncodeStream<S>
where
    S: Stream<Item = Result<Option<Vec<Message>>, ActixSendError>>,
{
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();

        if this.rx.is_some() {
            match Pin::new(&mut this.rx.as_mut().unwrap()).poll_next(cx) {
                Poll::Ready(Some(msg)) => {
                    return match this.codec.encode(msg, &mut this.buf) {
                        Ok(()) => Poll::Ready(Some(Ok(this.buf.split().freeze()))),
                        Err(e) => Poll::Ready(Some(Err(e.into()))),
                    };
                }
                Poll::Ready(None) => *this.rx = None,
                Poll::Pending => (),
            }
        }

        if let Some(msg) = this.queue.pop() {
            let poll = match this.codec.encode(msg, &mut this.buf) {
                Ok(()) => Poll::Ready(Some(Ok(this.buf.split().freeze()))),
                Err(e) => Poll::Ready(Some(Err(e.into()))),
            };
            cx.waker().wake_by_ref();
            return poll;
        }

        loop {
            match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Ready(Some(Ok(Some(ref mut msgs)))) => {
                    std::mem::swap(msgs, &mut this.queue);
                    return match this.queue.pop() {
                        Some(msg) => match this.codec.encode(msg, &mut this.buf) {
                            Ok(()) => Poll::Ready(Some(Ok(this.buf.split().freeze()))),
                            Err(e) => Poll::Ready(Some(Err(e.into()))),
                        },
                        None => Poll::Pending,
                    };
                }
                Poll::Ready(Some(Err(_))) => {
                    return Poll::Ready(Some(Err(actix_web::error::ErrorInternalServerError::<
                        &str,
                    >("test"))));
                }
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Ready(Some(Ok(None))) => continue,
            }
        }
    }
}
