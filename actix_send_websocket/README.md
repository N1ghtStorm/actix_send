### `actix_send_websocket` is a helper crate for managing websocket for [actix-web-v4](https://crates.io/crates/actix-web)

#### Example:
```rust
use actix_web::{get, App, Error, HttpRequest, HttpServer, Responder};
use actix_send_websocket::{Message, WebSocket};
use futures_util::StreamExt;

#[get("/")]
async fn ws(ws: WebSocket) -> impl Responder {
    // stream is the async iterator of incoming client websocket messages.
    // res is the response we return to client.
    // tx is a sender to push new websocket message to client response.
    let (stream, res) = ws.into_parts();

    // spawn the stream handling so we don't block the response to client.
    actix_web::rt::spawn(async move {
        actix_web::rt::pin!(stream);
        while let Some(Ok(msg)) = stream.next().await {
            match msg {
                // we echo text message and ping message to client.
                Message::Text(string) => stream.text(string),
                Message::Ping(bytes) => stream.pong(&bytes),
                Message::Close(reason) => {
                    stream.close(reason);
                    // force end the stream when we have a close message.
                    break;
                }
                // other types of message would be ignored
                _ => {},
            };
        }   
    });

    res
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Default behavior of websocket would be 5 seconds interval check for heart beat and 10 seconds for timeout.
    // Server sent ping is disabled by default.
    HttpServer::new(|| App::new().service(ws))
        .bind("127.0.0.1:8080")?
        .run()
        .await
}
```