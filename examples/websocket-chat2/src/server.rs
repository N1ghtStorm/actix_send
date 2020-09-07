use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use actix_send_websocket::{Message, WebSocketSender};

/// `ChatServer` manages chat rooms and responsible for coordinating chat
/// session. implementation is super primitive
pub struct ChatServer {
    sessions: HashMap<usize, WebSocketSender>,
    rooms: HashMap<String, HashSet<usize>>,
}

/// a thread safe pointer for `ChatServer` that is pass to `actix_web::App::data` as shared state.
#[derive(Clone, Default)]
pub struct SharedChatServer(Arc<Mutex<ChatServer>>);

impl SharedChatServer {
    pub fn get(&self) -> MutexGuard<'_, ChatServer> {
        self.0.lock().unwrap()
    }
}

impl Default for ChatServer {
    fn default() -> ChatServer {
        // default room
        let mut rooms = HashMap::new();
        rooms.insert("Main".to_owned(), HashSet::new());

        ChatServer {
            sessions: HashMap::new(),
            rooms,
        }
    }
}

impl ChatServer {
    // Send message to all users in the room
    pub fn send_message(&mut self, room: &str, message: &str, skip_id: usize) {
        let ids = self.rooms.get(room).map(|sessions| {
            sessions
                .iter()
                .filter(|id| **id != skip_id)
                .filter_map(|id| self.sessions.get(id).map(|addr| (addr, id)))
                .filter_map(|(addr, id)| {
                    // collect try_send error session ids.
                    addr.try_send(Message::Text(message.to_owned()))
                        .err()
                        .map(|_| *id)
                })
                .collect::<Vec<usize>>()
        });

        if let Some(ids) = ids {
            // try_send only fail when websocket connection is gone so disconnect them.
            ids.into_iter().for_each(|id| self.disconnect(id));
        }
    }

    pub fn connect(&mut self, id: usize, tx: WebSocketSender) {
        println!("Someone joined");

        // notify all users in same room
        self.send_message(&"Main".to_owned(), "Someone joined", 0);

        self.sessions.insert(id, tx);

        // auto join session to Main room
        self.join_room(id, "Main");
    }

    pub fn disconnect(&mut self, id: usize) {
        println!("Someone disconnected");

        // remove address
        if self.sessions.remove(&id).is_some() {
            self.exit_rooms(id);
        }
    }

    pub fn list_rooms(&self) -> Vec<String> {
        self.rooms.keys().cloned().collect()
    }

    fn join_room(&mut self, id: usize, name: &str) {
        self.rooms
            .entry(name.into())
            .or_insert_with(HashSet::new)
            .insert(id);
    }

    pub fn join(&mut self, id: usize, name: &str) {
        self.exit_rooms(id);

        self.join_room(id, name);

        self.send_message(&name, "Someone connected", id);
    }

    // remove id from all rooms and send message to other users.
    fn exit_rooms(&mut self, id: usize) {
        self.rooms
            .iter_mut()
            .filter_map(|(name, sessions)| {
                if sessions.remove(&id) {
                    Some(name.to_owned())
                } else {
                    None
                }
            })
            .collect::<Vec<String>>()
            .into_iter()
            // send message to other users
            .for_each(|room| self.send_message(&room, "Someone disconnected", 0))
    }
}
