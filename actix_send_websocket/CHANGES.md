## 0.2.1 - 2021-03-04
### Changed
- `WebSocketStream` must be pinned for collect the stream. This can be done with `actix_web::rt::pin`.
- Rework heartbeat and reduce memory usage. 

## 0.2.0 - 2021-02-28
### Changed
- update to `actix-web v4`
- `WebSocket::into_parts` return `(WebSocketStream, Response)`.

### Added
- `WebSocketStream` type for collect incoming request stream and sink response message.


## 0.1.0 - 2020-10-02
###Changed
- switch to `tokio::sync::mpsc` channel. In result `WebSocketSender::try_send` is deprecated and `WebSocketSender::xxx`
methods don't return futures anymore so no await is needed.

