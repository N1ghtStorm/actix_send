use actix_codec::{Decoder, Encoder};
use actix_http::ws::{hash_key, Codec};
pub use actix_http::ws::{CloseCode, CloseReason, Frame, HandshakeError, Message, ProtocolError};
use actix_send::prelude::*;
use actix_web::dev::HttpResponseBuilder;
use actix_web::error::{Error, PayloadError};
use actix_web::http::{header, Method, StatusCode};
use actix_web::{HttpRequest, HttpResponse};
use bytes::Bytes;
use futures_util::stream::{Stream, StreamExt};

pub async fn start<A, S>(
    builder: Builder<A>,
    req: &HttpRequest,
    stream: S,
) -> Result<HttpResponse, Error>
where
    A: Actor + Handler + 'static,
    A::Message: From<Result<Bytes, PayloadError>>,
    S: Stream<Item = Result<Bytes, PayloadError>> + Unpin + 'static,
    Result<Bytes, PayloadError>: MapResult<A::Result, Output = Option<WebsocketMessage>>,
{
    let mut res = handshake(&req)?;

    let addr = builder.start().await;

    let stream = addr
        .send_stream::<_, _, Result<Bytes, PayloadError>>(stream)
        .map(|res| match res {
            Ok(Some(msg)) => Some(Ok(msg)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        });

    let builder = EncodeActor::builder(|| async { EncodeActor });
    let enc_addr: Address<EncodeActor> = builder.start().await;

    let stream = enc_addr
        .send_skip_stream::<_, _, WebsocketEncodeMessage>(stream)
        .map(|res| match res {
            Ok(Ok(bytes)) => Ok(bytes),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(actix_http::error::ErrorInternalServerError::<&str>("lol")),
        });

    Ok(res.streaming(stream))
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
    let has_hdr = if let Some(hdr) = req.headers().get(&header::UPGRADE) {
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
    if !req.headers().contains_key(&header::SEC_WEBSOCKET_VERSION) {
        return Err(HandshakeError::NoVersionHeader);
    }
    let supported_ver = {
        if let Some(hdr) = req.headers().get(&header::SEC_WEBSOCKET_VERSION) {
            hdr == "13" || hdr == "8" || hdr == "7"
        } else {
            false
        }
    };
    if !supported_ver {
        return Err(HandshakeError::UnsupportedVersion);
    }

    // check client handshake for validity
    if !req.headers().contains_key(&header::SEC_WEBSOCKET_KEY) {
        return Err(HandshakeError::BadWebsocketKey);
    }
    let key = {
        let key = req.headers().get(&header::SEC_WEBSOCKET_KEY).unwrap();
        hash_key(key.as_ref())
    };

    // check requested protocols
    let protocol = req
        .headers()
        .get(&header::SEC_WEBSOCKET_PROTOCOL)
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
        response.header(&header::SEC_WEBSOCKET_PROTOCOL, protocol);
    }

    Ok(response)
}

pub struct WebsocketMessage;

impl From<Result<WebsocketMessage, ActixSendError>> for WebsocketEncodeMessage {
    fn from(res: Result<WebsocketMessage, ActixSendError>) -> Self {
        WebsocketEncodeMessage(res)
    }
}

#[actor(no_send)]
pub struct EncodeActor;

pub struct WebsocketEncodeMessage(Result<WebsocketMessage, ActixSendError>);

#[handler_v2(no_send)]
impl EncodeActor {
    async fn handle(&mut self, msg: WebsocketEncodeMessage) -> Result<Bytes, Error> {
        match msg.0 {
            Ok(msg) => (),
            Err(e) => (),
        }

        Ok(Bytes::new())
    }
}
