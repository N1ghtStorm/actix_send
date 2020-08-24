use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, MutexGuard};

use actix_files as fs;
use actix_send::prelude::*;
use actix_send_websocket::{Message, WebSocketMessage};
use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer};

mod server;

/// How often heartbeat pings are sent
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Entry point for our route
async fn chat_route(
    req: HttpRequest,
    stream: web::Payload,
    srv: web::Data<Arc<Mutex<server::ChatServer>>>,
) -> Result<HttpResponse, Error> {
    // generate a unique id for the session.
    let id = rand::random::<usize>();

    // construct a WsChatSession actor.
    let server = srv.get_ref().clone();
    // actor is built in async manner.
    let builder = WsChatSession::builder(move || {
        let server = server.clone();
        async move {
            WsChatSession {
                id,
                hb: Instant::now(),
                room: "Main".to_owned(),
                name: None,
                server,
            }
        }
    });
    // start the actor and get it's address.
    let address = builder.start().await;

    // simulate the heartbeat on server side.
    // in real world this would be the client's task.
    let addr = address.downgrade();
    actix_rt::spawn(async move {
        loop {
            let _ = actix_rt::time::delay_for(HEARTBEAT_INTERVAL).await;
            if let Some(addr) = addr.upgrade() {
                let _ = addr
                    .run(|session| {
                        Box::pin(async move {
                            session.hb = Instant::now();
                        })
                    })
                    .await;
            }
        }
    });

    // start the websocket handing with a sender where message can be pushed to websocket stream
    // and handled by WsChatSession::handle method.
    let (res, tx) = actix_send_websocket::start_with_tx(address, &req, stream).await?;

    // insert the sender and our session id to the chat server.
    srv.get_ref().lock().connect(id, tx);

    // return the response.
    Ok(res)
}

// definition of session actor.
#[actor(no_send)]
pub struct WsChatSession {
    /// unique session id
    pub id: usize,
    /// Client must send ping at least once per 10 seconds (CLIENT_TIMEOUT),
    /// otherwise we drop connection.
    pub hb: Instant,
    /// joined room
    pub room: String,
    /// peer name
    pub name: Option<String>,
    /// Chat server
    pub server: Arc<Mutex<server::ChatServer>>,
}

#[handler_v2(no_send)]
impl WsChatSession {
    // this method is called before session actor stop.
    #[on_stop]
    async fn this_happen_before_stop(&mut self) {
        self.get_server().disconnect(self.id);
    }

    // definition of handle method for incoming websocket stream message.
    // note we can return optional websocket messages directly in the method.
    async fn handle(&mut self, msg: WebSocketMessage) -> Option<Vec<Message>> {
        // if the heartbeat is beyond the timeout we send close message to client.
        if Instant::now().duration_since(self.hb) > CLIENT_TIMEOUT {
            return Some(vec![Message::Close(None)]);
        }

        let msg = msg.unwrap_or_else(|_| Message::Close(None));

        match msg {
            Message::Ping(msg) => {
                self.hb = Instant::now();
                Some(vec![Message::Pong(msg)])
            }
            Message::Pong(_) => {
                self.hb = Instant::now();
                None
            }
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
                            let rooms = self
                                .get_server()
                                .list_rooms()
                                .into_iter()
                                .map(|s| Message::Text(s.into()))
                                .collect();
                            Some(rooms)
                        }
                        "/join" => {
                            if v.len() == 2 {
                                self.room = v[1].to_owned();
                                self.get_server().join(self.id, &self.room);

                                Some(vec![Message::Text("joined".into())])
                            } else {
                                Some(vec![Message::Text(
                                    "!!! room name is required".into(),
                                )])
                            }
                        }
                        "/name" => {
                            if v.len() == 2 {
                                self.name = Some(v[1].to_owned());
                                None
                            } else {
                                Some(vec![Message::Text("!!! name is required".into())])
                            }
                        }
                        _ => Some(vec![Message::Text(format!(
                            "!!! unknown command: {:?}",
                            m
                        ))]),
                    }
                } else {
                    let msg = if let Some(ref name) = self.name {
                        format!("{}: {}", name, m)
                    } else {
                        m.to_owned()
                    };
                    // send message to chat server

                    self.get_server().send_message(
                        self.room.as_str(),
                        msg.as_str(),
                        self.id,
                    );

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
        }
    }
}

impl WsChatSession {
    fn get_server(&self) -> MutexGuard<'_, server::ChatServer> {
        self.server.lock()
    }
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();

    // Start chat server actor
    let server = Arc::new(Mutex::new(server::ChatServer::default()));

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
            .service(web::resource("/ws/").to(chat_route))
            // static resources
            .service(fs::Files::new("/static/", "static/"))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
