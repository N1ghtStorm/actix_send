//! A websocket service for actix-web framework.
//!
//! # Example:
//! ```rust,no_run
//! use actix_web::{get, App, Error, HttpRequest, HttpServer, Responder};
//! use actix_send_websocket::{Message, WebSocket};
//! use futures_util::{SinkExt, StreamExt};
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
//!         let mut should_end = false;
//!         while let Some(Ok(msg)) = stream.next().await {
//!             let msg = match msg {
//!                 // we echo text message and ping message to client.
//!                 Message::Text(string) => Some(Message::Text(string)),
//!                 Message::Ping(bytes) => Some(Message::Pong(bytes)),
//!                 Message::Close(reason) => {
//!                     should_end = true;
//!                     Some(Message::Close(reason))
//!                 }
//!                 // other types of message would be ignored
//!                 _ => None,
//!             };
//!             if let Some(msg) = msg {
//!                 // use the tx to send return message to client.
//!                 if tx.send(msg).await.is_err() {
//!                     break;
//!                 };  
//!             }    
//!             if should_end {
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
//!     .bind("127.0.0.1:8080")?
//!     .run()
//!     .await
//! }
//! ```

#![forbid(unsafe_code)]
#![forbid(unused_variables)]

use core::cell::RefCell;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;

use std::io;
use std::rc::Rc;
use std::time::Instant;

pub use actix_http::ws::{
    hash_key, CloseCode, CloseReason, Codec, Frame, HandshakeError, Message, ProtocolError,
};

use actix_codec::{Decoder, Encoder};
use actix_web::{
    dev::{HttpResponseBuilder, Payload},
    error::Error,
    http::{header, Method, StatusCode},
    web,
    web::{Bytes, BytesMut},
    FromRequest, HttpRequest, HttpResponse,
};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures_util::{
    future::{err, ok, Ready},
    stream::Stream,
};
use pin_project::pin_project;

/// config for WebSockets.
///
/// # example:
/// ```rust no_run
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
    heartbeat: Option<Duration>,
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

    /// Extract WsConfig config from app data. Check both `T` and `Data<T>`, in that order, and fall
    /// back to the default payload config.
    fn from_req(req: &HttpRequest) -> &Self {
        req.app_data::<Self>()
            .or_else(|| req.app_data::<web::Data<Self>>().map(|d| d.as_ref()))
            .unwrap_or_else(|| &DEFAULT_CONFIG)
    }
}

// Allow shared refs to default.
// ToDo: make Codec use const constructor.
const DEFAULT_CONFIG: WsConfig = WsConfig {
    codec: None,
    heartbeat: Some(Duration::from_secs(5)),
    timeout: Duration::from_secs(10),
};

impl Default for WsConfig {
    fn default() -> Self {
        DEFAULT_CONFIG.clone()
    }
}

/// extractor type for websocket.
pub struct WebSocket(
    pub DecodeStream,
    pub HttpResponse,
    pub UnboundedSender<Message>,
);

impl WebSocket {
    pub fn into_parts(self) -> (DecodeStream, HttpResponse, UnboundedSender<Message>) {
        (self.0, self.1, self.2)
    }
}

impl FromRequest for WebSocket {
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;
    type Config = WsConfig;

    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let mut res = match handshake(&req) {
            Ok(res) => res,
            Err(e) => return err(e.into()),
        };
        let cfg = Self::Config::from_req(req);
        let stream = payload.take();

        let (tx, rx) = futures_channel::mpsc::unbounded();

        let manager = DecodeStreamManager::new(cfg.heartbeat, cfg.timeout, tx.clone()).start();
        let codec = cfg.codec.unwrap_or_else(Codec::new);

        let decode = DecodeStream {
            manager,
            stream,
            codec,
            buf: Default::default(),
        };

        let encode = EncodeStream {
            rx,
            codec,
            buf: Default::default(),
        };

        let response = res.streaming(encode);

        ok(WebSocket(decode, response, tx))
    }
}

// decode incoming stream. eg: actix_web::web::Payload to a websocket Message.
#[pin_project]
pub struct DecodeStream {
    manager: DecodeStreamManager,
    #[pin]
    stream: Payload,
    codec: Codec,
    buf: BytesMut,
}

impl Stream for DecodeStream {
    // Decode error is packed with websocket Message and return as a Result.
    type Item = Result<Message, ProtocolError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();

        if !this.manager.is_closed() {
            loop {
                this = self.as_mut().project();
                match Pin::new(&mut this.stream).poll_next(cx) {
                    Poll::Ready(Some(Ok(chunk))) => {
                        this.buf.extend_from_slice(&chunk[..]);
                    }
                    Poll::Ready(None) => {
                        this.manager.set_close();
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
                if this.manager.is_closed() {
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
                    Frame::Ping(s) => {
                        // decode stream manager would handle ping message and close the connection
                        // if client failed to maintain heartbeat.
                        if this.manager.check_hb() {
                            Message::Ping(s)
                        } else {
                            Message::Close(Some(CloseCode::Normal.into()))
                        }
                    }
                    Frame::Pong(s) => Message::Pong(s),
                    Frame::Close(reason) => Message::Close(reason),
                    Frame::Continuation(item) => Message::Continuation(item),
                };
                Poll::Ready(Some(Ok(msg)))
            }
        }
    }
}

struct EncodeStream {
    rx: UnboundedReceiver<Message>,
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
    enable_heartbeat: bool,
    close_flag: Rc<RefCell<bool>>,
    interval: Option<Duration>,
    timeout: Duration,
    heartbeat: Rc<RefCell<Instant>>,
    tx: UnboundedSender<Message>,
}

impl DecodeStreamManager {
    fn new(interval: Option<Duration>, timeout: Duration, tx: UnboundedSender<Message>) -> Self {
        Self {
            enable_heartbeat: interval.is_some(),
            close_flag: Rc::new(RefCell::new(false)),
            interval,
            timeout,
            heartbeat: Rc::new(RefCell::new(Instant::now())),
            tx,
        }
    }

    fn start(self) -> Self {
        if let Some(interval) = self.interval {
            let this = self.clone();
            actix_web::rt::spawn(async move {
                loop {
                    actix_web::rt::time::delay_for(interval).await;
                    if Rc::strong_count(&this.close_flag) == 1 {
                        break;
                    }
                    if !this.check_hb() {
                        break;
                    }
                }
            });
        }

        self
    }

    fn set_close(&self) {
        *self.close_flag.borrow_mut() = true;
    }

    fn is_closed(&self) -> bool {
        *self.close_flag.borrow()
    }

    fn check_hb(&self) -> bool {
        if !self.enable_heartbeat {
            return true;
        }
        let now = Instant::now();
        let heartbeat = self.heartbeat.borrow();
        if now.duration_since(*heartbeat) > self.timeout {
            self.set_close();
            let _ = self
                .tx
                .unbounded_send(Message::Close(Some(CloseCode::Normal.into())));
            false
        } else {
            true
        }
    }
}

// Prepare `WebSocket` handshake response.
//
// This function returns handshake `HttpResponse`, ready to send to peer.
// It does not perform any IO.
fn handshake(req: &HttpRequest) -> Result<HttpResponseBuilder, HandshakeError> {
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
fn handshake_with_protocols(
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
