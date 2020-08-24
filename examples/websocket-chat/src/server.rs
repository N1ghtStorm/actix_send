use std::collections::{HashMap, HashSet};

use actix_send_websocket::Message;
use futures::channel::mpsc::UnboundedSender;

/// `ChatServer` manages chat rooms and responsible for coordinating chat
/// session. implementation is super primitive
pub struct ChatServer {
    sessions: HashMap<usize, UnboundedSender<Message>>,
    rooms: HashMap<String, HashSet<usize>>,
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
    /// Send message to all users in the room
    pub fn send_message(&self, room: &str, message: &str, skip_id: usize) {
        if let Some(sessions) = self.rooms.get(room) {
            for id in sessions {
                if *id != skip_id {
                    if let Some(addr) = self.sessions.get(id) {
                        let res = addr.unbounded_send(Message::Text(message.to_owned()));
                        assert!(res.is_ok());
                    }
                }
            }
        }
    }

    pub fn connect(&mut self, id: usize, tx: UnboundedSender<Message>) {
        println!("Someone joined");

        // notify all users in same room
        self.send_message(&"Main".to_owned(), "Someone joined", 0);

        // register session with random id

        self.sessions.insert(id, tx);

        // auto join session to Main room
        self.rooms
            .entry("Main".to_owned())
            .or_insert_with(HashSet::new)
            .insert(id);
    }

    pub fn disconnect(&mut self, id: usize) {
        println!("Someone disconnected");

        let mut rooms: Vec<String> = Vec::new();

        // remove address
        if self.sessions.remove(&id).is_some() {
            // remove session from all rooms
            for (name, sessions) in &mut self.rooms {
                if sessions.remove(&id) {
                    rooms.push(name.to_owned());
                }
            }
        }
        // send message to other users
        for room in rooms {
            self.send_message(&room, "Someone disconnected", 0);
        }
    }

    pub fn list_rooms(&self) -> Vec<&str> {
        let mut rooms = Vec::new();

        for key in self.rooms.keys() {
            rooms.push(key.as_str())
        }

        rooms
    }

    pub fn join(&mut self, id: usize, name: &str) {
        let mut rooms = Vec::new();

        // remove session from all rooms
        for (n, sessions) in &mut self.rooms {
            if sessions.remove(&id) {
                rooms.push(n.to_owned());
            }
        }
        // send message to other users
        for room in rooms {
            self.send_message(&room, "Someone disconnected", 0);
        }

        self.rooms
            .entry(name.to_owned())
            .or_insert_with(HashSet::new)
            .insert(id);

        self.send_message(&name, "Someone connected", id);
    }
}
