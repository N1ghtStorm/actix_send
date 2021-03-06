//! A websocket service for actix-web framework.
//!
//! # Example:
//! ```rust,no_run
//! use actix_web::{get, App, Error, HttpRequest, HttpServer, Responder};
//! use actix_send_websocket::{Message, WebSocket};
//! use futures_util::StreamExt;
//!
//! #[get("/")]
//! async fn ws(ws: WebSocket) -> impl Responder {
//!     // stream is the async iterator of incoming client websocket messages.
//!     // res is the response we return to client.
//!     // tx is a sender to push new websocket message to client response.
//!     let (stream, res) = ws.into_parts();
//!
//!     // spawn the stream handling so we don't block the response to client.
//!     actix_web::rt::spawn(async move {
//!         actix_web::rt::pin!(stream);
//!         while let Some(Ok(msg)) = stream.next().await {
//!             let result = match msg {
//!                 // we echo text message and ping message to client.
//!                 Message::Text(string) => stream.text(string),
//!                 Message::Ping(bytes) => stream.pong(&bytes),
//!                 Message::Close(reason) => {
//!                     let _ = stream.close(reason);
//!                     // force end the stream when we have a close message.
//!                     break;
//!                 }
//!                 // other types of message would be ignored
//!                 _ => Ok(()),
//!             };
//!             if result.is_err() {
//!                 // end the stream when the response is gone.
//!                 break;
//!             }
//!         }   
//!     });
//!
//!     res
//! }
//!
//! #[actix_web::main]
//! async fn main() -> std::io::Result<()> {
//!     HttpServer::new(|| App::new().service(ws))
//!         .bind("127.0.0.1:8080")?
//!         .run()
//!         .await
//! }
//! ```

#![forbid(unsafe_code)]
#![forbid(unused_variables)]

use core::convert::TryFrom;
use core::fmt::{Debug, Display};
use core::future::Future;
use core::ops::Deref;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;

use std::io;

use actix_codec::{Decoder, Encoder};
pub use actix_http::ws::{
    CloseCode, CloseReason, Codec, Frame, HandshakeError, Message, ProtocolError,
};
use actix_http::{
    error::{Error, ErrorInternalServerError},
    http::{header, Method, StatusCode},
    Payload, Response, ResponseBuilder,
};
use actix_web::{
    rt::time::{sleep, Instant, Sleep},
    web::{Bytes, BytesMut, Data},
    FromRequest, HttpRequest,
};
use bytestring::ByteString;
use futures_core::stream::Stream;

/// config for WebSockets.
///
/// # example:
/// ```rust,no_run
/// use actix_web::{App, HttpServer};
/// use actix_send_websocket::WsConfig;
///
/// #[actix_web::main]
/// async fn main() -> std::io::Result<()> {
///     HttpServer::new(||
///         App::new().app_data(WsConfig::new().disable_heartbeat())
///     )
///     .bind("127.0.0.1:8080")?
///     .run()
///     .await
/// }
/// ```
#[derive(Clone)]
pub struct WsConfig {
    codec: Option<Codec>,
    protocols: Option<Vec<String>>,
    heartbeat: Option<Duration>,
    server_send_heartbeat: bool,
    timeout: Duration,
}

impl WsConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set WebSockets protocol codec.
    pub fn codec(mut self, codec: Codec) -> Self {
        self.codec = Some(codec);
        self
    }

    /// Set specific protocol strings.
    /// `protocols` is a sequence of known protocols. On successful handshake,
    /// the returned response headers contain the first protocol in this list
    /// which the server also knows.
    pub fn protocols(mut self, protocols: Vec<String>) -> Self {
        self.protocols = Some(protocols);
        self
    }

    /// Set the heartbeat check interval.
    pub fn heartbeat(mut self, dur: Duration) -> Self {
        self.heartbeat = Some(dur);
        self
    }

    /// Set the timeout duration for client does not send Ping for too long.
    pub fn timeout(mut self, dur: Duration) -> Self {
        self.timeout = dur;
        self
    }

    /// Disable heartbeat check.
    pub fn disable_heartbeat(mut self) -> Self {
        self.heartbeat = None;
        self
    }

    /// Enable the heartbeat from Server side to Client.
    pub fn enable_server_send_heartbeat(mut self) -> Self {
        self.server_send_heartbeat = true;
        self
    }

    /// Extract WsConfig config from app data. Check both `T` and `Data<T>`, in that order, and fall
    /// back to the default payload config.
    fn from_req(req: &HttpRequest) -> &Self {
        req.app_data::<Self>()
            .or_else(|| req.app_data::<Data<Self>>().map(|d| d.as_ref()))
            .unwrap_or(&DEFAULT_CONFIG)
    }
}

// Allow shared refs to default.
// ToDo: make Codec use const constructor.
const DEFAULT_CONFIG: WsConfig = WsConfig {
    codec: None,
    protocols: None,
    heartbeat: Some(Duration::from_secs(5)),
    server_send_heartbeat: false,
    timeout: Duration::from_secs(10),
};

impl Default for WsConfig {
    fn default() -> Self {
        DEFAULT_CONFIG
    }
}

/// extractor type for websocket.
pub struct WebSocket(pub WebSocketStream<Payload>, pub Response);

impl WebSocket {
    #[inline]
    pub fn into_parts(self) -> (WebSocketStream<Payload>, Response) {
        (self.0, self.1)
    }
}

impl FromRequest for WebSocket {
    type Config = WsConfig;
    type Error = Error;
    type Future = WebSocketFuture<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let cfg = Self::Config::from_req(req);

        let res = match cfg.protocols {
            Some(ref protocols) => {
                let protocols = protocols.iter().map(|f| f.as_str()).collect::<Vec<_>>();
                handshake_with_protocols(&req, &protocols)
            }
            None => handshake_with_protocols(&req, &[]),
        };

        match res {
            Ok(mut res) => {
                let (stream, encode) = split_stream(cfg, payload.take());
                WebSocketFuture::new(Ok(WebSocket(stream, res.streaming(encode))))
            }
            Err(e) => WebSocketFuture::new(Err(e.into())),
        }
    }
}

pin_project_lite::pin_project! {
    pub struct WebSocketFuture<T> {
        res: Option<T>
    }
}

impl<T> WebSocketFuture<T> {
    fn new(res: T) -> Self {
        Self { res: Some(res) }
    }
}

impl<T> Future for WebSocketFuture<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.project().res.take().unwrap())
    }
}

/// split stream into decode/encode streams and a sender that can send item to encode stream.
pub fn split_stream<S>(cfg: &WsConfig, stream: S) -> (WebSocketStream<S>, EncodeStream) {
    let (tx, rx) = channel::channel();
    let tx = WebSocketSender(tx);

    let codec = cfg.codec.unwrap_or_else(Codec::new);

    let (timer, interval) = match cfg.heartbeat {
        Some(dur) => (Some(sleep(dur)), dur),
        None => (None, Duration::from_secs(1)),
    };

    (
        WebSocketStream {
            decode: DecodeStream {
                stream,
                timer,
                hb: HeartBeat::new(interval, cfg.timeout, cfg.server_send_heartbeat),
                tx: tx.clone(),
                codec,
                read_buf: BytesMut::new(),
            },
            tx,
        },
        EncodeStream {
            rx,
            codec,
            write_buf: BytesMut::new(),
        },
    )
}

pin_project_lite::pin_project! {
    pub struct WebSocketStream<S> {
        #[pin]
        decode: DecodeStream<S>,
        tx: WebSocketSender,
    }
}

impl<S> Deref for WebSocketStream<S> {
    type Target = WebSocketSender;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl<S> WebSocketStream<S> {
    /// Get a reference of the sender.
    /// `WebSocketSender` can be used to push response message to client.
    #[inline]
    pub fn sender(&self) -> &WebSocketSender {
        &self.tx
    }

    /// Owned version of `WebSocketStream::sender` API.
    #[inline]
    pub fn sender_owned(&self) -> WebSocketSender {
        self.tx.clone()
    }
}

impl<E, S> Stream for WebSocketStream<S>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Debug + Display,
{
    // Decode error is packed with websocket Message and return as a Result.
    type Item = Result<Message, ProtocolError>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().decode.poll_next(cx)
    }
}

// decode incoming stream.
pin_project_lite::pin_project! {
     struct DecodeStream<S> {
        #[pin]
        stream: S,
        #[pin]
        timer: Option<Sleep>,
        hb: HeartBeat,
        tx: WebSocketSender,
        codec: Codec,
        read_buf: BytesMut,
    }
}

impl<E, S> Stream for DecodeStream<S>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Debug + Display,
{
    // Decode error is packed with websocket Message and return as a Result.
    type Item = Result<Message, ProtocolError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();

        loop {
            if !this.read_buf.is_empty() {
                if let Some(frame) = this.codec.decode(this.read_buf)? {
                    let msg = match frame {
                        Frame::Text(data) => {
                            Message::Text(ByteString::try_from(data).map_err(|e| {
                                ProtocolError::Io(io::Error::new(
                                    io::ErrorKind::Other,
                                    format!("{}", e),
                                ))
                            })?)
                        }
                        Frame::Binary(data) => Message::Binary(data),
                        Frame::Ping(s) => {
                            this.hb.update();
                            Message::Ping(s)
                        }
                        Frame::Pong(s) => {
                            this.hb.update();
                            Message::Pong(s)
                        }
                        Frame::Close(reason) => Message::Close(reason),
                        Frame::Continuation(item) => Message::Continuation(item),
                    };
                    return Poll::Ready(Some(Ok(msg)));
                }
            }

            match this.stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    this.read_buf.extend_from_slice(&chunk[..]);
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(ProtocolError::Io(io::Error::new(
                        io::ErrorKind::Other,
                        format!("{}", e),
                    )))));
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => {
                    if let Some(mut timer) = this.timer.as_pin_mut() {
                        if timer.as_mut().poll(cx).is_ready() {
                            if this.hb.set(timer.as_mut()) {
                                let _ = timer.poll(cx);
                                if this.hb.server_send_heartbeat {
                                    // TODO: work out this timeout error
                                    this.tx.ping(b"ping").map_err(|e| {
                                        ProtocolError::Io(io::Error::new(
                                            io::ErrorKind::Other,
                                            format!("{}", e),
                                        ))
                                    })?;
                                }
                            } else {
                                this.tx.close(Some(CloseCode::Normal.into())).map_err(|e| {
                                    ProtocolError::Io(io::Error::new(
                                        io::ErrorKind::Other,
                                        format!("{}", e),
                                    ))
                                })?;
                                return Poll::Ready(None);
                            }
                        }
                    }

                    return Poll::Pending;
                }
            }
        }
    }
}

pub struct EncodeStream {
    rx: channel::Receiver<Message>,
    codec: Codec,
    write_buf: BytesMut,
}

impl Stream for EncodeStream {
    type Item = Result<Bytes, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            match Pin::new(&mut this.rx).poll_recv(cx) {
                Poll::Ready(Some(msg)) => {
                    if let Message::Close(_) = msg {
                        // Close the receiver on close message.
                        this.rx.close();
                    }
                    this.codec.encode(msg, &mut this.write_buf)?
                }
                Poll::Pending => {
                    return if !this.write_buf.is_empty() {
                        Poll::Ready(Some(Ok(this.write_buf.split().freeze())))
                    } else {
                        Poll::Pending
                    }
                }
                Poll::Ready(None) => {
                    return if !this.write_buf.is_empty() {
                        // Self wake up to properly end the stream
                        cx.waker().wake_by_ref();
                        Poll::Ready(Some(Ok(this.write_buf.split().freeze())))
                    } else {
                        Poll::Ready(None)
                    };
                }
            }
        }
    }
}

#[derive(Clone)]
struct HeartBeat {
    server_send_heartbeat: bool,
    interval: Duration,
    timeout: Duration,
    deadline: Instant,
}

impl HeartBeat {
    #[inline]
    fn new(interval: Duration, timeout: Duration, server_send_heartbeat: bool) -> Self {
        Self {
            server_send_heartbeat,
            interval,
            timeout,
            deadline: Instant::now() + timeout,
        }
    }

    fn update(&mut self) {
        self.deadline = Instant::now() + self.timeout;
    }

    fn set(&self, hb: Pin<&mut Sleep>) -> bool {
        let now = hb.deadline();
        if now >= self.deadline {
            false
        } else {
            hb.reset(now + self.interval);
            true
        }
    }
}

#[derive(Clone)]
pub struct WebSocketSender(channel::Sender<Message>);

impl WebSocketSender {
    /// Send text frame
    #[inline]
    pub fn text(&self, text: impl Into<ByteString>) -> Result<(), Error> {
        self.send(Message::Text(text.into()))
    }

    /// Send binary frame
    #[inline]
    pub fn binary(&self, data: impl Into<Bytes>) -> Result<(), Error> {
        self.send(Message::Binary(data.into()))
    }

    /// Send ping frame
    #[inline]
    pub fn ping(&self, message: &[u8]) -> Result<(), Error> {
        self.send(Message::Ping(Bytes::copy_from_slice(message)))
    }

    /// Send pong frame
    #[inline]
    pub fn pong(&self, message: &[u8]) -> Result<(), Error> {
        self.send(Message::Pong(Bytes::copy_from_slice(message)))
    }

    /// Send close frame
    #[inline]
    pub fn close(&self, reason: Option<CloseReason>) -> Result<(), Error> {
        self.send(Message::Close(reason))
    }
}

/// Prepare `WebSocket` handshake response.
///
/// This function returns handshake `HttpResponse`, ready to send to peer.
/// It does not perform any IO.
pub fn handshake(req: &HttpRequest) -> Result<ResponseBuilder, HandshakeError> {
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
) -> Result<ResponseBuilder, HandshakeError> {
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
        actix_http::ws::hash_key(key.as_ref())
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

    let mut response = Response::build(StatusCode::SWITCHING_PROTOCOLS)
        .upgrade("websocket")
        .insert_header((header::TRANSFER_ENCODING, "chunked"))
        .insert_header((header::SEC_WEBSOCKET_ACCEPT, &key[..]))
        .take();

    if let Some(protocol) = protocol {
        response.insert_header((header::SEC_WEBSOCKET_PROTOCOL, protocol));
    }

    Ok(response)
}

mod channel {
    use super::{Error, ErrorInternalServerError, Message};

    pub(super) use tokio::sync::mpsc::{UnboundedReceiver as Receiver, UnboundedSender as Sender};

    pub(super) fn channel<T>() -> (Sender<T>, Receiver<T>) {
        tokio::sync::mpsc::unbounded_channel()
    }

    impl super::WebSocketSender {
        /// send the message asynchronously.
        #[inline]
        pub fn send(&self, message: Message) -> Result<(), Error> {
            self.0.send(message).map_err(ErrorInternalServerError)
        }

        #[deprecated(
            note = "channel has been moved to use tokio::sync::mpsc so try_send is removed"
        )]
        pub fn try_send(
            &self,
            message: Message,
        ) -> Result<(), tokio::sync::mpsc::error::SendError<Message>> {
            self.0.send(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::{header, Method};
    use actix_web::test::TestRequest;

    #[test]
    fn test_handshake() {
        let req = TestRequest::default()
            .method(Method::POST)
            .to_http_request();
        assert_eq!(
            HandshakeError::GetMethodRequired,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default().to_http_request();
        assert_eq!(
            HandshakeError::NoWebsocketUpgrade,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .insert_header((header::UPGRADE, header::HeaderValue::from_static("test")))
            .to_http_request();
        assert_eq!(
            HandshakeError::NoWebsocketUpgrade,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .to_http_request();
        assert_eq!(
            HandshakeError::NoConnectionUpgrade,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .insert_header((
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            ))
            .to_http_request();
        assert_eq!(
            HandshakeError::NoVersionHeader,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .insert_header((
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("5"),
            ))
            .to_http_request();
        assert_eq!(
            HandshakeError::UnsupportedVersion,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .insert_header((
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            ))
            .to_http_request();
        assert_eq!(
            HandshakeError::BadWebsocketKey,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .insert_header((
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_KEY,
                header::HeaderValue::from_static("13"),
            ))
            .to_http_request();

        assert_eq!(
            StatusCode::SWITCHING_PROTOCOLS,
            handshake(&req).unwrap().finish().status()
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .insert_header((
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_KEY,
                header::HeaderValue::from_static("13"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_PROTOCOL,
                header::HeaderValue::from_static("graphql"),
            ))
            .to_http_request();

        let protocols = ["graphql"];

        assert_eq!(
            StatusCode::SWITCHING_PROTOCOLS,
            handshake_with_protocols(&req, &protocols)
                .unwrap()
                .finish()
                .status()
        );
        assert_eq!(
            Some(&header::HeaderValue::from_static("graphql")),
            handshake_with_protocols(&req, &protocols)
                .unwrap()
                .finish()
                .headers()
                .get(&header::SEC_WEBSOCKET_PROTOCOL)
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .insert_header((
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_KEY,
                header::HeaderValue::from_static("13"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_PROTOCOL,
                header::HeaderValue::from_static("p1, p2, p3"),
            ))
            .to_http_request();

        let protocols = vec!["p3", "p2"];

        assert_eq!(
            StatusCode::SWITCHING_PROTOCOLS,
            handshake_with_protocols(&req, &protocols)
                .unwrap()
                .finish()
                .status()
        );
        assert_eq!(
            Some(&header::HeaderValue::from_static("p2")),
            handshake_with_protocols(&req, &protocols)
                .unwrap()
                .finish()
                .headers()
                .get(&header::SEC_WEBSOCKET_PROTOCOL)
        );

        let req = TestRequest::default()
            .insert_header((
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            ))
            .insert_header((
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_KEY,
                header::HeaderValue::from_static("13"),
            ))
            .insert_header((
                header::SEC_WEBSOCKET_PROTOCOL,
                header::HeaderValue::from_static("p1,p2,p3"),
            ))
            .to_http_request();

        let protocols = vec!["p3", "p2"];

        assert_eq!(
            StatusCode::SWITCHING_PROTOCOLS,
            handshake_with_protocols(&req, &protocols)
                .unwrap()
                .finish()
                .status()
        );
        assert_eq!(
            Some(&header::HeaderValue::from_static("p2")),
            handshake_with_protocols(&req, &protocols)
                .unwrap()
                .finish()
                .headers()
                .get(&header::SEC_WEBSOCKET_PROTOCOL)
        );
    }
}
