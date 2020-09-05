use std::time::Duration;

use actix_files as fs;
use actix_send_websocket::{CloseCode, Message, WebSocket, WsConfig};
use actix_web::{
    get,
    web::{self, Data},
    App, HttpResponse, HttpServer, Responder,
};
use futures_util::StreamExt;

use crate::server::SharedChatServer;

mod server;

/// How often heartbeat pings are sent
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

// session information stores in request.
pub struct WsChatSession {
    /// unique session id
    pub id: usize,
    /// joined room
    pub room: String,
    /// peer name
    pub name: Option<String>,
}

/// Entry point for our route
#[get("/ws/")]
async fn chat_route(server: Data<SharedChatServer>, websocket: WebSocket) -> impl Responder {
    // stream is the async iterator for incoming websocket messages.
    // res is the response to client.
    // tx is the sender to add message to response.
    let (mut stream, res, mut tx) = websocket.into_parts();

    // construct a session.
    let mut session = WsChatSession {
        id: rand::random::<usize>(),
        room: "Main".to_string(),
        name: None,
    };

    // insert id and sender to chat server. It can be used to send message directly to client from
    // other threads and/or websocket connections.
    server.get().connect(session.id, tx.clone());

    // spawn the message handling future so we don't block our response to client.
    actix_web::rt::spawn(async move {
        while let Some(res) = stream.next().await {
            let msg = res.unwrap_or_else(|_| Message::Close(Some(CloseCode::Protocol.into())));

            let res = match msg {
                Message::Ping(msg) => tx.pong(&msg).await,
                Message::Pong(_) => Ok(()),
                Message::Text(text) => {
                    let m = text.trim();
                    // we check for /sss type of messages
                    if m.starts_with('/') {
                        let v: Vec<&str> = m.splitn(2, ' ').collect();
                        match v[0] {
                            "/list" => {
                                // Send ListRooms message to chat server and wait for
                                // response
                                println!("List rooms");
                                let rooms = server
                                    .get()
                                    .list_rooms()
                                    .into_iter()
                                    .map(|text| text.into())
                                    .collect::<Vec<String>>();

                                for room in rooms.into_iter() {
                                    if tx.text(room).await.is_err() {
                                        break;
                                    }
                                }

                                Ok(())
                            }
                            "/join" => {
                                let text = if v.len() == 2 {
                                    server.get().join(session.id, &session.room);
                                    "joined"
                                } else {
                                    "!!! room name is required"
                                };

                                tx.text(text).await
                            }
                            "/name" => {
                                let msg = if v.len() == 2 {
                                    session.name = Some(v[1].to_owned());

                                    format!("new name is {}", v[1])
                                } else {
                                    "!!! name is required".into()
                                };

                                tx.text(msg).await
                            }
                            _ => tx.text(format!("!!! unknown command: {:?}", m)).await,
                        }
                    } else {
                        let msg = if let Some(ref name) = session.name {
                            format!("{}: {}", name, m)
                        } else {
                            m.to_owned()
                        };
                        // send message to chat server
                        server
                            .get()
                            .send_message(&session.room, msg.as_str(), session.id);

                        Ok(())
                    }
                }
                Message::Binary(_) => {
                    println!("Unexpected binary");
                    Ok(())
                }
                Message::Close(reason) => {
                    // close could either be sent by the client or the built in heartbeat manager.
                    // so we should echo the message to client and then end the stream.
                    let _ = tx.close(reason).await;
                    break;
                }
                Message::Continuation(_) => {
                    let _ = tx.close(None).await;
                    break;
                }
                Message::Nop => Ok(()),
            };

            if res.is_err() {
                break;
            }
        }

        // we are disconnected. remove the tx of session.
        server.get().disconnect(session.id);
    });

    res
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Start chat server
    let server = SharedChatServer::default();

    // Create Http server with websocket support
    HttpServer::new(move || {
        App::new()
            .data(server.clone())
            .app_data(
                WsConfig::new()
                    .heartbeat(HEARTBEAT_INTERVAL)
                    .timeout(CLIENT_TIMEOUT)
                    .disable_heartbeat(),
            )
            // redirect to websocket.html
            .service(web::resource("/").route(web::get().to(|| {
                HttpResponse::Found()
                    .header("LOCATION", "/static/websocket.html")
                    .finish()
            })))
            // websocket route
            .service(chat_route)
            // static resources
            .service(fs::Files::new("/static/", "static/"))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
