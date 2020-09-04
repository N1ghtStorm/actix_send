use std::time::Duration;

use actix_files as fs;
use actix_send_websocket::{Message, ProtocolError, WebSockets};
use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer};
use futures_channel::mpsc::UnboundedSender;

use crate::server::SharedChatServer;
use actix_web::web::Data;

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

// function called when a websocket connection is established.
async fn on_call(req: HttpRequest, tx: UnboundedSender<Message>) -> Result<(), Error> {
    // generate a session and insert it into http request.
    let id = rand::random::<usize>();
    let session = WsChatSession {
        id,
        room: "Main".to_string(),
        name: None,
    };

    // insert id and sender to chat server. It can be used to send message directly to client from
    // other threads and/or websocket connections.
    req.app_data::<Data<SharedChatServer>>()
        .unwrap()
        .get()
        .connect(id, tx);

    req.extensions_mut().insert(session);
    Ok(())
}

// function called when a websocket connection is closing
fn on_stop(req: &HttpRequest) {
    if let Some(session) = req.extensions_mut().get_mut::<WsChatSession>() {
        req.app_data::<Data<SharedChatServer>>()
            .unwrap()
            .get()
            .disconnect(session.id);
    }
}

/// Entry point for our route
async fn chat_route(
    req: HttpRequest,
    msg: Result<Message, ProtocolError>,
) -> Result<Option<Vec<Message>>, Error> {
    let msg = msg.unwrap_or_else(|_| Message::Close(None));

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
                        let rooms = req
                            .app_data::<Data<SharedChatServer>>()
                            .unwrap()
                            .get()
                            .list_rooms()
                            .into_iter()
                            .map(|s| Message::Text(s.into()))
                            .collect();
                        Some(rooms)
                    }
                    "/join" => {
                        if v.len() == 2 {
                            if let Some(session) = req.extensions_mut().get_mut::<WsChatSession>() {
                                session.room = v[1].to_owned();

                                req.app_data::<Data<SharedChatServer>>()
                                    .unwrap()
                                    .get()
                                    .join(session.id, &session.room);
                                Some(vec![Message::Text("joined".into())])
                            } else {
                                Some(vec![Message::Close(None)])
                            }
                        } else {
                            Some(vec![Message::Text("!!! room name is required".into())])
                        }
                    }
                    "/name" => {
                        if v.len() == 2 {
                            if let Some(session) = req.extensions_mut().get_mut::<WsChatSession>() {
                                session.name = Some(v[1].to_owned());

                                Some(vec![Message::Text(format!("new name is {}", v[1]))])
                            } else {
                                Some(vec![Message::Close(None)])
                            }
                        } else {
                            Some(vec![Message::Text("!!! name is required".into())])
                        }
                    }
                    _ => Some(vec![Message::Text(format!("!!! unknown command: {:?}", m))]),
                }
            } else {
                if let Some(session) = req.extensions_mut().get_mut::<WsChatSession>() {
                    let msg = if let Some(ref name) = session.name {
                        format!("{}: {}", name, m)
                    } else {
                        m.to_owned()
                    };
                    // send message to chat server
                    req.app_data::<Data<SharedChatServer>>()
                        .unwrap()
                        .get()
                        .send_message(&session.room, msg.as_str(), session.id);
                }
                None
            }
        }
        Message::Binary(_) => {
            println!("Unexpected binary");
            None
        }
        Message::Close(reason) => Some(vec![Message::Close(reason)]),
        Message::Continuation(_) => Some(vec![Message::Close(None)]),
        Message::Nop => None,
    };

    Ok(res)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Start chat server
    let server = SharedChatServer::default();

    // Create Http server with websocket support
    HttpServer::new(move || {
        App::new()
            .data(server.clone())
            // redirect to websocket.html
            .service(web::resource("/").route(web::get().to(|| {
                HttpResponse::Found()
                    .header("LOCATION", "/static/websocket.html")
                    .finish()
            })))
            // websocket
            .service(
                WebSockets::new("/ws/")
                    .timeout(CLIENT_TIMEOUT)
                    .heartbeat(HEARTBEAT_INTERVAL)
                    // remove this to enable proper heartbeat.
                    .disable_heartbeat()
                    .to(chat_route)
                    .on_call(on_call)
                    .on_stop(on_stop),
            )
            // static resources
            .service(fs::Files::new("/static/", "static/"))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
