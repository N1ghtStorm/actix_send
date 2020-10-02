## 0.1.0 - 2020-10-02
###Changed
- switch to `tokio::sync::mpsc` channel. In result `WebSocketSender::try_send` is deprecated and `WebSocketSender::xxx`
methods don't return futures anymore so no await is needed.

