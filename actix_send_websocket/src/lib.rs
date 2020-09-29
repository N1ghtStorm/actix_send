//! A websocket service for actix-web framework.
//!
//! # Example:
//! ```rust,no_run
//! use actix_web::{get, App, Error, HttpRequest, HttpServer, Responder};
//! use actix_send_websocket::{Message, WebSocket};
//!
//! #[get("/")]
//! async fn ws(ws: WebSocket) -> impl Responder {
//!     // stream is the async iterator of incoming client websocket messages.
//!     // res is the response we return to client.
//!     // tx is a sender to push new websocket message to client response.
//!     let (mut stream, res, mut tx) = ws.into_parts();
//!
//!     // spawn the stream handling so we don't block the response to client.
//!     actix_web::rt::spawn(async move {
//!         while let Some(Ok(msg)) = stream.next().await {
//!             let result = match msg {
//!                 // we echo text message and ping message to client.
//!                 Message::Text(string) => tx.text(string),
//!                 Message::Ping(bytes) => tx.pong(&bytes),
//!                 Message::Close(reason) => {
//!                     let _ = tx.close(reason);
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

use core::cell::RefCell;
use core::fmt::{Debug, Display};
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;

use std::io;
use std::rc::Rc;
use std::time::Instant;

use actix_codec::{Decoder, Encoder};
pub use actix_http::ws::{
    CloseCode, CloseReason, Codec, Frame, HandshakeError, Message, ProtocolError,
};
use actix_http::{
    error::{Error, ErrorInternalServerError},
    http::{header, Method, StatusCode},
    Payload, Response, ResponseBuilder,
};
use actix_utils::task::LocalWaker;
use actix_web::{
    rt,
    web::{Bytes, BytesMut, Data},
    FromRequest, HttpRequest,
};
use futures_util::{
    // ToDo: move to std::future::{ready, Ready} when 1.48 hits stable.
    future::{ready, Ready},
    // ToDo: move to std::future::stream whenever it's stable.
    stream::Stream,
};
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
            .unwrap_or_else(|| &DEFAULT_CONFIG)
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
        DEFAULT_CONFIG.clone()
    }
}

/// extractor type for websocket.
pub struct WebSocket(pub DecodeStream<Payload>, pub Response, pub WebSocketSender);

impl WebSocket {
    #[inline]
    pub fn into_parts(self) -> (DecodeStream<Payload>, Response, WebSocketSender) {
        (self.0, self.1, self.2)
    }
}

impl FromRequest for WebSocket {
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;
    type Config = WsConfig;

    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let cfg = Self::Config::from_req(req);

        let protocols = match cfg.protocols {
            Some(ref protocols) => protocols.iter().map(|f| f.as_str()).collect::<Vec<_>>(),
            None => Vec::with_capacity(0),
        };

        match handshake_with_protocols(&req, &protocols) {
            Ok(mut res) => {
                let (decode, encode, tx) = split_stream(cfg, payload.take());
                ready(Ok(WebSocket(decode, res.streaming(encode), tx)))
            }
            Err(e) => ready(Err(e.into())),
        }
    }
}

/// split stream into decode/encode streams and a sender that can send item to encode stream.
pub fn split_stream<S>(
    cfg: &WsConfig,
    stream: S,
) -> (DecodeStream<S>, EncodeStream, WebSocketSender) {
    let (tx, rx) = channel::channel();
    let tx = WebSocketSender(tx);

    let codec = cfg.codec.unwrap_or_else(Codec::new);

    (
        DecodeStream {
            manager: DecodeStreamManager::new(
                cfg.heartbeat,
                cfg.timeout,
                cfg.server_send_heartbeat,
                tx.clone(),
            )
            .start(),
            stream,
            codec,
            buf: Default::default(),
        },
        EncodeStream {
            rx,
            codec,
            buf: Default::default(),
        },
        tx,
    )
}

// decode incoming stream.
pin_project_lite::pin_project! {
    pub struct DecodeStream<S> {
        manager: DecodeStreamManager,
        #[pin]
        stream: S,
        codec: Codec,
        buf: BytesMut,
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

        // resolve when heartbeat future is gone.
        if this.manager.enable_heartbeat {
            if this.manager.is_heartbeat_future_closed() {
                return Poll::Ready(None);
            }

            this.manager.register_decode_stream_waker(cx);
        }

        loop {
            if !this.buf.is_empty() {
                match this.codec.decode(this.buf) {
                    Ok(Some(frm)) => {
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
                            Frame::Ping(s) => {
                                this.manager.update_heartbeat_ping();
                                Message::Ping(s)
                            }
                            Frame::Pong(s) => {
                                this.manager.update_heartbeat_pong();
                                Message::Pong(s)
                            }
                            Frame::Close(reason) => Message::Close(reason),
                            Frame::Continuation(item) => Message::Continuation(item),
                        };
                        return Poll::Ready(Some(Ok(msg)));
                    }
                    Ok(None) => (),
                    Err(e) => return Poll::Ready(Some(Err(e))),
                }
            }

            match this.stream.poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    this.buf.extend(&chunk);
                    this = self.as_mut().project();
                }
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(ProtocolError::Io(io::Error::new(
                        io::ErrorKind::Other,
                        format!("{}", e),
                    )))));
                }
                Poll::Ready(None) => return Poll::Ready(None),
            }
        }
    }
}

#[allow(clippy::should_implement_trait)]
impl<E, S> DecodeStream<S>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: Debug + Display,
{
    pub fn next(&mut self) -> futures_util::stream::Next<'_, Self> {
        futures_util::stream::StreamExt::next(self)
    }

    pub fn try_next(&mut self) -> futures_util::stream::TryNext<'_, Self> {
        futures_util::stream::TryStreamExt::try_next(self)
    }
}

pub struct EncodeStream {
    rx: channel::Receiver<Message>,
    codec: Codec,
    buf: BytesMut,
}

impl Stream for EncodeStream {
    type Item = Result<Bytes, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        match Pin::new(&mut this.rx).poll_next(cx) {
            Poll::Ready(Some(msg)) => match this.codec.encode(msg, &mut this.buf) {
                Ok(()) => Poll::Ready(Some(Ok(this.buf.split().freeze()))),
                Err(e) => Poll::Ready(Some(Err(e.into()))),
            },
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Clone)]
struct DecodeStreamManager {
    decode_stream_waker: Rc<RefCell<LocalWaker>>,
    heartbeat_future_waker: Rc<RefCell<LocalWaker>>,
    enable_heartbeat: bool,
    server_send_heartbeat: bool,
    interval: Option<Duration>,
    timeout: Duration,
    heartbeat: Rc<RefCell<Instant>>,
    tx: WebSocketSender,
}

// try wake up decode stream and heartbeat future when dropping manager.
impl Drop for DecodeStreamManager {
    fn drop(&mut self) {
        self.decode_stream_waker.borrow().wake();
        self.heartbeat_future_waker.borrow().wake();
    }
}

impl DecodeStreamManager {
    #[inline]
    fn new(
        interval: Option<Duration>,
        timeout: Duration,
        server_send_heartbeat: bool,
        tx: WebSocketSender,
    ) -> Self {
        Self {
            decode_stream_waker: Rc::new(RefCell::new(Default::default())),
            heartbeat_future_waker: Rc::new(RefCell::new(Default::default())),
            enable_heartbeat: interval.is_some(),
            server_send_heartbeat,
            interval,
            timeout,
            heartbeat: Rc::new(RefCell::new(Instant::now())),
            tx,
        }
    }

    #[inline]
    fn start(self) -> Self {
        if let Some(interval) = self.interval {
            let manager = self.clone();
            rt::spawn(async move {
                loop {
                    let is_shutdown = HeartbeatFuture::new(interval, &manager).await;

                    if is_shutdown {
                        break;
                    }

                    if manager.server_send_heartbeat && manager.tx.ping(b"ping").is_err() {
                        break;
                    }
                }
            });
        }

        self
    }

    fn register_decode_stream_waker(&self, cx: &mut Context<'_>) {
        self.decode_stream_waker.borrow_mut().register(cx.waker());
    }

    fn register_heartbeat_future_waker(&self, cx: &mut Context<'_>) {
        self.heartbeat_future_waker
            .borrow_mut()
            .register(cx.waker());
    }

    fn is_timeout(&self) -> bool {
        let heartbeat = self.heartbeat.borrow();
        if Instant::now().duration_since(*heartbeat) > self.timeout {
            // send close message to client when hb check is failed.
            let _ = self.tx.send(Message::Close(Some(CloseCode::Normal.into())));
            true
        } else {
            false
        }
    }

    fn update_heartbeat_ping(&self) {
        if self.enable_heartbeat {
            self.update_heartbeat();
        }
    }

    // Determine if we update heartbeat when a Pong message is received.
    fn update_heartbeat_pong(&self) {
        if self.server_send_heartbeat {
            self.update_heartbeat();
        }
    }

    fn update_heartbeat(&self) {
        *self.heartbeat.borrow_mut() = Instant::now();
    }

    fn is_heartbeat_future_closed(&self) -> bool {
        Rc::strong_count(&self.heartbeat_future_waker) == 1
    }

    fn is_decode_stream_closed(&self) -> bool {
        Rc::strong_count(&self.decode_stream_waker) == 1
    }
}

struct HeartbeatFuture<'a> {
    manager: &'a DecodeStreamManager,
    delay: rt::time::Delay,
}

impl<'a> HeartbeatFuture<'a> {
    fn new(dur: Duration, manager: &'a DecodeStreamManager) -> Self {
        Self {
            manager,
            delay: rt::time::delay_for(dur),
        }
    }
}

impl Future for HeartbeatFuture<'_> {
    type Output = bool;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // return true when heartbeat timed out/ EncodeStream channel is closed / DecodeStream is
        // closed.
        if this.manager.is_timeout() || this.manager.is_decode_stream_closed() {
            return Poll::Ready(true);
        }

        this.manager.register_heartbeat_future_waker(cx);

        Pin::new(&mut this.delay).poll(cx).map(|_| false)
    }
}

#[derive(Clone)]
pub struct WebSocketSender(channel::Sender<Message>);

impl WebSocketSender {
    /// Send text frame
    #[inline]
    pub fn text(&self, text: impl Into<String>) -> Result<(), Error> {
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

// Prepare `WebSocket` handshake response.
//
// This function returns handshake `HttpResponse`, ready to send to peer.
// It does not perform any IO.
pub fn handshake(req: &HttpRequest) -> Result<ResponseBuilder, HandshakeError> {
    handshake_with_protocols(req, &[])
}

// Prepare `WebSocket` handshake response.
//
// This function returns handshake `HttpResponse`, ready to send to peer.
// It does not perform any IO.
//
// `protocols` is a sequence of known protocols. On successful handshake,
// the returned response headers contain the first protocol in this list
// which the server also knows.
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
        .header(header::TRANSFER_ENCODING, "chunked")
        .header(header::SEC_WEBSOCKET_ACCEPT, key.as_str())
        .take();

    if let Some(protocol) = protocol {
        response.header(header::SEC_WEBSOCKET_PROTOCOL, protocol);
    }

    Ok(response)
}

#[cfg(feature = "send")]
#[cfg(not(feature = "no-send"))]
mod channel {
    use super::{Error, ErrorInternalServerError, Message};

    pub(super) use tokio::sync::mpsc::{UnboundedReceiver as Receiver, UnboundedSender as Sender};

    pub(super) fn channel<T>() -> (Sender<T>, Receiver<T>) {
        tokio::sync::mpsc::unbounded_channel()
    }

    impl super::WebSocketSender {
        /// send the message asynchronously.
        pub fn send(&self, message: Message) -> Result<(), Error> {
            self.0.send(message).map_err(ErrorInternalServerError)
        }

        /// send the message synchronously. The send would fail only when the receive part is closed.
        ///
        /// The returned Error type is `futures_channel::mpsc::TrySendError`.
        ///
        /// *. It's suggested call `into_inner` and get the message that failed to sent instead  of
        /// importing the error type from said crate.
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

#[cfg(feature = "no-send")]
#[cfg(not(feature = "send"))]
mod channel {
    use super::{Error, ErrorInternalServerError, Message};

    pub(super) use actix_utils::mpsc::{Receiver, Sender};

    pub(super) fn channel<T>() -> (Sender<T>, Receiver<T>) {
        actix_utils::mpsc::channel()
    }

    impl super::WebSocketSender {
        /// send the message asynchronously.
        pub fn send(&mut self, message: Message) -> Result<(), Error> {
            self.0.send(message).map_err(ErrorInternalServerError)
        }

        /// send the message synchronously. The send would fail only when the receive part is closed.
        ///
        /// The returned Error type is `actix_utils::mpsc::SendError`.
        ///
        /// *. It's suggested call `into_inner` and get the message that failed to sent instead  of
        /// importing the error type from said crate.
        #[deprecated(note = "please use channel::send instead.")]
        pub fn try_send(
            &self,
            message: Message,
        ) -> Result<(), actix_utils::mpsc::SendError<Message>> {
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
            .header(header::UPGRADE, header::HeaderValue::from_static("test"))
            .to_http_request();
        assert_eq!(
            HandshakeError::NoWebsocketUpgrade,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .to_http_request();
        assert_eq!(
            HandshakeError::NoConnectionUpgrade,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .to_http_request();
        assert_eq!(
            HandshakeError::NoVersionHeader,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .header(
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("5"),
            )
            .to_http_request();
        assert_eq!(
            HandshakeError::UnsupportedVersion,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .header(
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            )
            .to_http_request();
        assert_eq!(
            HandshakeError::BadWebsocketKey,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .header(
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            )
            .header(
                header::SEC_WEBSOCKET_KEY,
                header::HeaderValue::from_static("13"),
            )
            .to_http_request();

        assert_eq!(
            StatusCode::SWITCHING_PROTOCOLS,
            handshake(&req).unwrap().finish().status()
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .header(
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            )
            .header(
                header::SEC_WEBSOCKET_KEY,
                header::HeaderValue::from_static("13"),
            )
            .header(
                header::SEC_WEBSOCKET_PROTOCOL,
                header::HeaderValue::from_static("graphql"),
            )
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
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .header(
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            )
            .header(
                header::SEC_WEBSOCKET_KEY,
                header::HeaderValue::from_static("13"),
            )
            .header(
                header::SEC_WEBSOCKET_PROTOCOL,
                header::HeaderValue::from_static("p1, p2, p3"),
            )
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
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .header(
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            )
            .header(
                header::SEC_WEBSOCKET_KEY,
                header::HeaderValue::from_static("13"),
            )
            .header(
                header::SEC_WEBSOCKET_PROTOCOL,
                header::HeaderValue::from_static("p1,p2,p3"),
            )
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
