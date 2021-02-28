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

