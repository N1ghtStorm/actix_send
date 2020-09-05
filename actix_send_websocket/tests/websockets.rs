use core::time::Duration;

use actix_send_websocket::{CloseCode, Frame, Message, WebSocket, WsConfig};
use actix_web::{get, test, web::Bytes, App, Responder};
use futures_util::{SinkExt, StreamExt};

#[get("/")]
async fn handler(WebSocket(mut stream, res, mut tx): WebSocket) -> impl Responder {
    actix_web::rt::spawn(async move {
        let mut should_end = false;
        while let Some(Ok(msg)) = stream.next().await {
            let msg = match msg {
                Message::Text(str) => Some(Message::Text(str)),
                Message::Binary(bytes) => Some(Message::Binary(bytes)),
                Message::Ping(bytes) => Some(Message::Pong(bytes)),
                Message::Close(reason) => {
                    should_end = true;
                    Some(Message::Close(reason))
                }
                _ => None,
            };

            if let Some(msg) = msg {
                if tx.send(msg).await.is_err() {
                    break;
                }
            }

            if should_end {
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

    framed
        .send(Message::Text("text".to_string()))
        .await
        .unwrap();

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

    actix_rt::time::delay_for(Duration::from_secs(2)).await;

    let item = framed.next().await.unwrap().unwrap();
    assert_eq!(item, Frame::Close(Some(CloseCode::Normal.into())));

    framed
        .send(Message::Text("text".to_string()))
        .await
        .unwrap();

    let item = framed.next().await;
    assert!(item.is_none());
}
