use core::time::Duration;

use actix_send_websocket::{CloseCode, Frame, Message, WebSocket, WsConfig};
use actix_web::{get, test, web::Bytes, App, Responder};
use futures_util::{SinkExt, StreamExt};

#[get("/")]
async fn handler(WebSocket(stream, res): WebSocket) -> impl Responder {
    actix_web::rt::spawn(async move {
        actix_web::rt::pin!(stream);
        while let Some(Ok(msg)) = stream.next().await {
            let result = match msg {
                Message::Text(str) => stream.text(str),
                Message::Binary(bytes) => stream.binary(bytes),
                Message::Ping(bytes) => stream.pong(&bytes),
                Message::Close(reason) => {
                    let _ = stream.close(reason);
                    break;
                }
                _ => Ok(()),
            };

            if result.is_err() {
                break;
            }
        }
    });

    res
}

#[actix_rt::test]
async fn test_websocket() {
    let mut srv = test::start(|| {
        App::new()
            .app_data(WsConfig::new().disable_heartbeat())
            .service(handler)
    });

    let mut framed = srv.ws().await.unwrap();

    framed.send(Message::Text("text".into())).await.unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Text(Bytes::from_static(b"text")));

    framed.send(Message::Binary("text".into())).await.unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Binary(Bytes::from_static(b"text")));

    framed.send(Message::Ping("text".into())).await.unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Pong(Bytes::copy_from_slice(b"text")));

    framed
        .send(Message::Close(Some(CloseCode::Normal.into())))
        .await
        .unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Close(Some(CloseCode::Normal.into())));
}

#[actix_rt::test]
async fn test_heartbeat() {
    let mut srv = test::start(|| {
        App::new()
            .app_data(
                WsConfig::new()
                    .heartbeat(Duration::from_millis(500))
                    .timeout(Duration::from_secs(1)),
            )
            .service(handler)
    });

    let mut framed = srv.ws().await.unwrap();

    framed.send(Message::Ping("text".into())).await.unwrap();
    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Pong(Bytes::copy_from_slice(b"text")));

    actix_rt::time::sleep(Duration::from_secs(2)).await;

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Close(Some(CloseCode::Normal.into())));

    framed.send(Message::Text("text".into())).await.unwrap();

    let item = framed.next().await;
    assert!(item.is_none());
}

#[actix_rt::test]
async fn test_server_send_heartbeat() {
    let mut srv = test::start(|| {
        App::new()
            .app_data(
                WsConfig::new()
                    .heartbeat(Duration::from_millis(600))
                    .enable_server_send_heartbeat()
                    .timeout(Duration::from_secs(1)),
            )
            .service(handler)
    });

    let mut framed = srv.ws().await.unwrap();

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Ping(Bytes::from_static(b"ping")));

    actix_rt::time::sleep(Duration::from_secs(2)).await;

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Close(Some(CloseCode::Normal.into())));

    let item = framed.next().await;
    assert!(item.is_none());
}
