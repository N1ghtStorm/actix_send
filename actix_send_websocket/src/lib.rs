//! A websocket service for actix-web framework.
//!
//! # Example:
//! ```rust,no_run
//! use actix_web::{App, Error, HttpRequest, HttpServer};
//! use actix_send_websocket::{Message, ProtocolError, WebSockets};
//! use futures_channel::mpsc::UnboundedSender;
//!
//! // the websocket message handler. incoming message are from the client and return optional
//! // message to client.
//! async fn ws(req: HttpRequest, message: Result<Message, ProtocolError>) -> Result<Option<Vec<Message>>, Error> {
//!     let message = message.unwrap_or(Message::Close(None));
//!     match message {
//!         // we echo text message and ping message to client.
//!         Message::Text(string) => Ok(Some(vec![Message::Text(string)])),
//!         Message::Ping(bytes) => Ok(Some(vec![Message::Pong(bytes)])),
//!         Message::Close(reason) => Ok(Some(vec![Message::Close(reason)])),
//!         // other types of message would be ignored
//!         _ => Ok(None),
//!     }   
//! }
//!
//! // this would called before the websocket service is called.
//! // it gives access to the request and a sender that can send message directly to the
//! // client.
//! async fn on_call(req: HttpRequest, tx: UnboundedSender<Message>) -> Result<(), Error> {
//!     Ok(())
//! }
//!
//! // this would called when a websocket connection is closing.
//! fn on_stop(req: &HttpRequest) {
//! }
//!
//! #[actix_web::main]
//! async fn main() -> std::io::Result<()> {
//!     HttpServer::new(||
//!         App::new()
//!             .service(
//!                 WebSockets::new("/")
//!                     .to(ws)
//!                     .on_call(on_call)
//!                     .on_stop(on_stop)
//!             )
//!     )
//!     .bind("127.0.0.1:8080")?
//!     .run()
//!     .await
//! }
//! ```

#![forbid(unsafe_code)]
#![forbid(unused_variables)]

use core::cell::RefCell;
use core::future::Future;
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
use actix_service::{
    boxed::{BoxService, BoxServiceFactory},
    Service, ServiceFactory,
};
use actix_web::{
    dev::{
        AppService, HttpResponseBuilder, HttpServiceFactory, ResourceDef, ServiceRequest,
        ServiceResponse,
    },
    error::{Error, PayloadError},
    http::{header, Method, StatusCode},
    web::{Bytes, BytesMut},
    HttpRequest, HttpResponse,
};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures_util::{
    future::{err, ok, Either, LocalBoxFuture, Ready},
    stream::Stream,
    FutureExt,
};
use pin_project::{pin_project, pinned_drop};

type HttpService = BoxService<ServiceRequest, ServiceResponse, Error>;
type HttpNewService = BoxServiceFactory<(), ServiceRequest, ServiceResponse, Error, ()>;

pub struct WebSockets {
    codec: Codec,
    heartbeat: Option<Duration>,
    timeout: Duration,
    path: String,
    default: Rc<RefCell<Option<Rc<HttpNewService>>>>,
    handler: WebSocketsHandler,
    on_call: Option<WebSocketsOnCallHandler>,
    on_stop: Option<WebSocketsOnStopHandler>,
}

impl WebSockets {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            codec: Codec::new(),
            heartbeat: Some(Duration::from_secs(5)),
            timeout: Duration::from_secs(10),
            path: path.into(),
            default: Rc::new(RefCell::new(None)),
            handler: WebSocketsHandler {
                handler: Rc::new(|_, _| async { Ok(None) }),
            },
            on_call: None,
            on_stop: None,
        }
    }

    pub fn codec(mut self, codec: Codec) -> Self {
        self.codec = codec;
        self
    }

    /// Set the heartbeat interval.
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

    /// websocket handler.
    pub fn to<F, R>(mut self, handler: F) -> Self
    where
        F: Fn(HttpRequest, Result<Message, ProtocolError>) -> R + 'static,
        R: Future<Output = Result<Option<Vec<Message>>, Error>> + 'static,
    {
        self.handler = WebSocketsHandler {
            handler: Rc::new(handler),
        };

        self
    }

    /// Handler function which would called before a service is called.
    pub fn on_call<F, R>(mut self, on_call: F) -> Self
    where
        F: Fn(HttpRequest, UnboundedSender<Message>) -> R + 'static,
        R: Future<Output = Result<(), Error>> + 'static,
    {
        self.on_call = Some(WebSocketsOnCallHandler {
            handler: Rc::new(on_call),
        });
        self
    }

    /// Handler function which would called when websocket connection is shutting down.
    pub fn on_stop<F>(mut self, on_stop: F) -> Self
    where
        F: Fn(&HttpRequest) + 'static,
    {
        self.on_stop = Some(WebSocketsOnStopHandler {
            handler: Rc::new(on_stop),
        });
        self
    }
}

#[derive(Clone)]
struct WebSocketsHandler {
    handler: Rc<dyn WebSocketsHandlerTrait>,
}

trait WebSocketsHandlerTrait {
    fn handle(
        &self,
        req: HttpRequest,
        msg: Result<Message, ProtocolError>,
    ) -> LocalBoxFuture<'static, Result<Option<Vec<Message>>, Error>>;
}

impl<F, R> WebSocketsHandlerTrait for F
where
    F: Fn(HttpRequest, Result<Message, ProtocolError>) -> R,
    R: Future<Output = Result<Option<Vec<Message>>, Error>> + 'static,
{
    fn handle(
        &self,
        req: HttpRequest,
        msg: Result<Message, ProtocolError>,
    ) -> LocalBoxFuture<'static, Result<Option<Vec<Message>>, Error>> {
        Box::pin(self(req, msg))
    }
}

#[derive(Clone)]
struct WebSocketsOnCallHandler {
    handler: Rc<dyn WebSocketsOnCallHandlerTrait>,
}

trait WebSocketsOnCallHandlerTrait {
    fn handle(
        &self,
        req: HttpRequest,
        sender: UnboundedSender<Message>,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>>>>;
}

impl<F, R> WebSocketsOnCallHandlerTrait for F
where
    F: Fn(HttpRequest, UnboundedSender<Message>) -> R,
    R: Future<Output = Result<(), Error>> + 'static,
{
    fn handle(
        &self,
        req: HttpRequest,
        sender: UnboundedSender<Message>,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>>>> {
        Box::pin(self(req, sender))
    }
}

#[derive(Clone)]
struct WebSocketsOnStopHandler {
    handler: Rc<dyn WebSocketsOnStopHandlerTrait>,
}

trait WebSocketsOnStopHandlerTrait {
    fn handle(&self, req: &HttpRequest);
}

impl<F> WebSocketsOnStopHandlerTrait for F
where
    F: Fn(&HttpRequest),
{
    fn handle(&self, req: &HttpRequest) {
        (self)(req);
    }
}

#[derive(Clone)]
struct DecodeStreamManager {
    enable_heartbeat: bool,
    close_flag: Rc<RefCell<bool>>,
    interval: Option<Duration>,
    timeout: Duration,
    heartbeat: Rc<RefCell<Instant>>,
}

impl DecodeStreamManager {
    fn new(interval: Option<Duration>, timeout: Duration) -> Self {
        Self {
            enable_heartbeat: interval.is_some(),
            close_flag: Rc::new(RefCell::new(false)),
            interval,
            timeout,
            heartbeat: Rc::new(RefCell::new(Instant::now())),
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
            false
        } else {
            true
        }
    }
}

impl HttpServiceFactory for WebSockets {
    fn register(self, config: &mut AppService) {
        if self.default.borrow().is_none() {
            *self.default.borrow_mut() = Some(config.default_service());
        }
        let rdef = if config.is_root() {
            ResourceDef::root_prefix(&self.path)
        } else {
            ResourceDef::prefix(&self.path)
        };
        config.register_service(rdef, None, self, None)
    }
}

impl ServiceFactory for WebSockets {
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = WebSocketsService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let mut srv = WebSocketsService {
            codec: self.codec,
            heartbeat: self.heartbeat,
            timeout: self.timeout,
            default: None,
            handler: self.handler.clone(),
            on_call: self.on_call.clone(),
            on_stop: self.on_stop.clone(),
        };

        if let Some(ref default) = *self.default.borrow() {
            default
                .new_service(())
                .map(move |result| match result {
                    Ok(default) => {
                        srv.default = Some(default);
                        Ok(srv)
                    }
                    Err(_) => Err(()),
                })
                .boxed_local()
        } else {
            ok(srv).boxed_local()
        }
    }
}

pub struct WebSocketsService {
    codec: Codec,
    heartbeat: Option<Duration>,
    timeout: Duration,
    default: Option<HttpService>,
    handler: WebSocketsHandler,
    on_call: Option<WebSocketsOnCallHandler>,
    on_stop: Option<WebSocketsOnStopHandler>,
}

impl Service for WebSocketsService {
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    #[allow(clippy::type_complexity)]
    type Future = Either<
        Ready<Result<Self::Response, Self::Error>>,
        LocalBoxFuture<'static, Result<Self::Response, Self::Error>>,
    >;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        let (req, payload) = req.into_parts();

        // do handshake first.
        let mut res = match handshake(&req) {
            Ok(res) => res,
            Err(e) => return Either::Left(err(e.into())),
        };

        // construct a channel if we have a on_call function. the channel is used to push message directly
        // to response(EncodeStream) stream.
        let mut on_call = None;
        let rx = match self.on_call.as_ref() {
            Some(call) => {
                let (tx, rx) = futures_channel::mpsc::unbounded();
                on_call = Some(call.handler.handle(req.clone(), tx));
                Some(rx)
            }
            None => None,
        };

        let manager = DecodeStreamManager::new(self.heartbeat, self.timeout).start();

        let decode_stream = DecodeStream {
            manager,
            stream: payload,
            codec: self.codec,
            buf: BytesMut::new(),
        };

        // handler stream would take the on_stop function and call it when it's dropping.
        let handler_stream = HandlerStream {
            on_stop: self.on_stop.clone(),
            stream: decode_stream,
            request: req.clone(),
            handler: self.handler.clone(),
            fut: None,
        };

        let encode_stream = EncodeStream {
            rx,
            stream: handler_stream,
            buf: BytesMut::new(),
            codec: self.codec,
            queue: Vec::new(),
        };

        let res = res.streaming(encode_stream);

        match on_call {
            Some(on_call) => Either::Right(Box::pin(async move {
                on_call.await?;
                Ok(ServiceResponse::new(req, res))
            })),
            None => Either::Left(ok(ServiceResponse::new(req, res))),
        }
    }
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
#[pin_project]
struct DecodeStream<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    manager: DecodeStreamManager,
    #[pin]
    stream: S,
    codec: Codec,
    buf: BytesMut,
}

impl<S> Stream for DecodeStream<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
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
                            Message::Close(None)
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

#[pin_project(PinnedDrop)]
struct HandlerStream<S>
where
    // messages come as an option.
    S: Stream<Item = Result<Message, ProtocolError>>,
{
    handler: WebSocketsHandler,
    on_stop: Option<WebSocketsOnStopHandler>,
    #[pin]
    stream: S,
    request: HttpRequest,
    fut: Option<LocalBoxFuture<'static, Result<Option<Vec<Message>>, Error>>>,
}

#[pinned_drop]
impl<S> PinnedDrop for HandlerStream<S>
where
    S: Stream<Item = Result<Message, ProtocolError>>,
{
    fn drop(self: Pin<&mut Self>) {
        let this = self.project();
        if let Some(on_stop) = this.on_stop.as_mut() {
            on_stop.handler.handle(&this.request);
        }
    }
}

impl<S> Stream for HandlerStream<S>
where
    S: Stream<Item = Result<Message, ProtocolError>>,
{
    // Decode error is packed with websocket Message and return as a Result.
    type Item = Result<Option<Vec<Message>>, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();

        if let Some(fut) = this.fut.as_mut() {
            return match fut.as_mut().poll(cx) {
                Poll::Ready(res) => {
                    *this.fut = None;
                    Poll::Ready(Some(res))
                }
                Poll::Pending => return Poll::Pending,
            };
        }

        match Pin::new(&mut this.stream).poll_next(cx) {
            Poll::Ready(Some(msg)) => {
                let mut fut = this.handler.handler.handle(this.request.clone(), msg);
                match fut.as_mut().poll(cx) {
                    Poll::Ready(res) => Poll::Ready(Some(res)),
                    Poll::Pending => {
                        *this.fut = Some(fut);
                        Poll::Pending
                    }
                }
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[pin_project]
struct EncodeStream<S>
where
    // messages come as an option.
    S: Stream<Item = Result<Option<Vec<Message>>, Error>>,
{
    rx: Option<UnboundedReceiver<Message>>,
    #[pin]
    stream: S,
    codec: Codec,
    buf: BytesMut,
    queue: Vec<Message>,
}

impl<S> EncodeStream<S>
where
    S: Stream<Item = Result<Option<Vec<Message>>, Error>>,
{
    fn poll_encode(self: Pin<&mut Self>, msg: Message) -> Poll<Option<Result<Bytes, Error>>> {
        let mut this = self.project();
        match this.codec.encode(msg, &mut this.buf) {
            Ok(()) => Poll::Ready(Some(Ok(this.buf.split().freeze()))),
            Err(e) => Poll::Ready(Some(Err(e.into()))),
        }
    }
}

impl<S> Stream for EncodeStream<S>
where
    S: Stream<Item = Result<Option<Vec<Message>>, Error>>,
{
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();

        if this.rx.is_some() {
            match Pin::new(&mut this.rx.as_mut().unwrap()).poll_next(cx) {
                Poll::Ready(Some(msg)) => return self.poll_encode(msg),
                Poll::Ready(None) => *this.rx = None,
                Poll::Pending => (),
            }
        }

        if let Some(msg) = this.queue.pop() {
            let poll = self.poll_encode(msg);
            cx.waker().wake_by_ref();
            return poll;
        }

        loop {
            match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Ready(Some(Ok(Some(ref mut msgs)))) => {
                    std::mem::swap(msgs, &mut this.queue);
                    return match this.queue.pop() {
                        Some(msg) => {
                            let poll = self.poll_encode(msg);
                            cx.waker().wake_by_ref();
                            poll
                        }
                        None => Poll::Pending,
                    };
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Ready(Some(Ok(None))) => continue,
            }
        }
    }
}
