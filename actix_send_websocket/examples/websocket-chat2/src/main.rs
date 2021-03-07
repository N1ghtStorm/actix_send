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
    let (stream, res) = websocket.into_parts();

    // spawn the message handling future so we don't block our response to client.
    actix_web::rt::spawn(async move {
        // construct a session. 
        // session is tasks with register/deresgiter this ws connection to chat server.
        // other interaction with chat server and other ws connection is also managed 
        // by session struct. 
        let mut session = WsChatSession::new(&*server, stream.sender());

        // pin stream for zero overhead handling.
        // Box::pin(stream) would also work but at the cost of heap allocation.
        actix_web::rt::pin!(stream);

        // iter through the incoming stream of messages.
        while let Some(Ok(res)) = stream.next().await {
            match msg {
                Message::Ping(msg) => stream.pong(&msg),
                Message::Text(text) => {
                    let m = text.trim();
                    // we check for /sss type of messages
                    if m.starts_with('/') {
                        let v: Vec<&str> = m.splitn(2, ' ').collect();
                        match v[0] {
                            "/list" => {
                                println!("List rooms");
                                let rooms = session.server.get().list_rooms();

                                for room in rooms.into_iter() {
                                    stream.text(room);
                                }
                            }
                            "/join" => {
                                let text = if v.len() == 2 {
                                    session.server.get().join(session.id, &session.room);
                                    "joined"
                                } else {
                                    "!!! room name is required"
                                };

                                stream.text(text)
                            }
                            "/name" => {
                                let msg = if v.len() == 2 {
                                    session.name = Some(v[1].to_owned());

                                    format!("new name is {}", v[1])
                                } else {
                                    "!!! name is required".into()
                                };

                                stream.text(msg)
                            }
                            _ => stream.text(format!("!!! unknown command: {:?}", m)),
                        }
                    } else {
                        let msg = if let Some(ref name) = session.name {
                            format!("{}: {}", name, m)
                        } else {
                            m.to_owned()
                        };
                        // send message with chat server
                        session
                            .server
                            .get()
                            .send_message(&session.room, msg.as_str(), session.id);
                    }
                }
                Message::Binary(_) => println!("Unexpected binary"),
                Message::Close(reason) => {
                    stream.close(reason);
                    return;
                }
                Message::Continuation(_) => {
                    stream.close(None);
                    return;
                }
                Message::Nop | Message::Pong(_) => {}
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
                    // default setting does not enable server side heartbeat to client.
                    .enable_server_send_heartbeat(),
            )
            // redirect to websocket.html
            .service(web::resource("/").route(web::get().to(|| {
                HttpResponse::Found()
                    .insert_header(("LOCATION", "/static/websocket.html"))
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
