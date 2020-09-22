use std::time::Duration;

use actix_files as fs;
use actix_send_websocket::{CloseCode, Message, WebSocket, WebSocketSender, WsConfig};
use actix_web::{
    get,
    web::{self, Data},
    App, HttpResponse, HttpServer, Responder,
};

use crate::server::SharedChatServer;

mod server;

/// How often heartbeat pings are sent
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

// session information stores in route handler.
pub struct WsChatSession<'a> {
    /// unique session id
    pub id: usize,
    /// joined room
    pub room: String,
    /// peer name
    pub name: Option<String>,
    /// a reference of shared chat server. So session can remove the tx from server when dropped.
    pub server: &'a SharedChatServer,
}

impl<'a> WsChatSession<'a> {
    fn new(server: &'a SharedChatServer, tx: WebSocketSender) -> Self {
        let id = rand::random::<usize>();

        // insert id and sender to chat server. It can be used to send message directly to client from
        // other threads and/or websocket connections.
        server.get().connect(id, tx);

        Self {
            id,
            room: "Main".to_string(),
            name: None,
            server,
        }
    }
}

/// Notify server disconnect when dropping.
impl Drop for WsChatSession<'_> {
    fn drop(&mut self) {
        self.server.get().disconnect(self.id);
    }
}

/// Entry point for our route
#[get("/ws/")]
async fn chat_route(server: Data<SharedChatServer>, websocket: WebSocket) -> impl Responder {
    // stream is the async iterator for incoming websocket messages.
    // res is the response to client.
    // tx is the sender to add message to response.
    let (mut stream, res, mut tx) = websocket.into_parts();

    // spawn the message handling future so we don't block our response to client.
    actix_web::rt::spawn(async move {
        // construct a session.
        let mut session = WsChatSession::new(&*server, tx.clone());

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
                                println!("List rooms");
                                let rooms = server.get().list_rooms();

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
                        // send message with chat server
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

            // send is failed because client response is gone so we should end the stream.
            if res.is_err() {
                break;
            }
        }
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
            // pass shared chat server as app state.
            .data(server.clone())
            // config for websocket behavior. if not presented a default one would be used.
            .app_data(
                WsConfig::new()
                    .heartbeat(HEARTBEAT_INTERVAL)
                    .timeout(CLIENT_TIMEOUT)
                    // heartbeat is disabled for testing purpose.
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
