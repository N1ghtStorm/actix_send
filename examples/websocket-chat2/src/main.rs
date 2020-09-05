use std::time::Duration;

use actix_files as fs;
use actix_send_websocket::{CloseCode, Message, WebSocketStream, WsConfig};
use actix_web::{
    get,
    web::{self, Data},
    App, HttpRequest, HttpResponse, HttpServer, Responder,
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
async fn chat_route(server: Data<SharedChatServer>, websocket: WebSocketStream) -> impl Responder {
    // stream is the async iterator for incoming websocket messages.
    // res is the response to client.
    // tx is the sender to add message to response.
    let (mut stream, res, tx) = websocket.into_parts();

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
                Message::Ping(msg) => Some(vec![Message::Pong(msg)]),
                Message::Pong(_) => None,
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
                                    .map(|s| Message::Text(s.into()))
                                    .collect();
                                Some(rooms)
                            }
                            "/join" => {
                                if v.len() == 2 {
                                    server.get().join(session.id, &session.room);
                                    Some(vec![Message::Text("joined".into())])
                                } else {
                                    Some(vec![Message::Text("!!! room name is required".into())])
                                }
                            }
                            "/name" => {
                                if v.len() == 2 {
                                    session.name = Some(v[1].to_owned());

                                    Some(vec![Message::Text(format!("new name is {}", v[1]))])
                                } else {
                                    Some(vec![Message::Text("!!! name is required".into())])
                                }
                            }
                            _ => Some(vec![Message::Text(format!("!!! unknown command: {:?}", m))]),
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

                        None
                    }
                }
                Message::Binary(_) => {
                    println!("Unexpected binary");
                    None
                }
                Message::Close(_) => break,
                Message::Continuation(_) => Some(vec![Message::Close(None)]),
                Message::Nop => None,
            };

            if let Some(msg) = res {
                for msg in msg.into_iter() {
                    let _ = tx.unbounded_send(msg);
                }
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
