use std::collections::HashMap;
use std::thread::ThreadId;

use actix_send::prelude::*;

use crate::cloneable_actor::CloneAbleActor;
use crate::shared_actor::{Message2, Message2Res, ShareableActor};

#[actor_mod]
pub mod shared_actor {
    use super::*;

    #[actor]
    pub struct ShareableActor {
        pub state: usize,
        pub info: HashMap<ThreadId, usize>,
    }

    #[message(result = "u8")]
    pub struct Message1;

    #[message(result = "Vec<Message2Res>")]
    pub struct Message2;

    pub struct Message2Res {
        pub thread_id: ThreadId,
        pub count: usize,
    }

    #[handler]
    impl Handler for ShareableActor {
        async fn handle(&mut self, _msg: Message1) -> u8 {
            self.state += 1;
            let id = std::thread::current().id();
            let key = self.info.get_mut(&id);
            match key {
                Some(key) => *key += 1,
                None => {
                    self.info.insert(id, 1);
                }
            };
            1
        }
    }

    #[handler]
    impl Handler for ShareableActor {
        async fn handle(&mut self, _msg: Message2) -> Vec<Message2Res> {
            println!("Actor have handled a total of {} messages\r\n", self.state);
            self.info
                .iter()
                .map(|(thread_id, count)| Message2Res {
                    thread_id: *thread_id,
                    count: *count,
                })
                .collect()
        }
    }
}

#[actor_mod]
pub mod cloneable_actor {
    use super::*;

    #[actor]
    #[derive(Clone)]
    pub struct CloneAbleActor {
        pub state: usize,
    }

    #[message(result = "usize")]
    pub struct Message1;

    #[handler]
    impl Handler for CloneAbleActor {
        async fn handle(&mut self, _msg: Message1) -> usize {
            self.state += 1;
            self.state
        }
    }
}

#[tokio::main]
async fn main() {
    // build and start shareable actors.
    // Actors share the same address, the same actor state.
    let actor = ShareableActor::create(|| ShareableActor {
        state: 0,
        info: Default::default(),
    });

    let address = actor.build().num(12).start();

    // send messages
    for _i in 0..1_000 {
        // Both shared_actor and cloneable_actor have the same type Message1
        // and we can specific call the one we want with a type path.
        let _: u8 = address.send(shared_actor::Message1).await.unwrap();
    }

    let info: Vec<Message2Res> = address.send(Message2).await.unwrap();

    for i in info.into_iter() {
        println!("{:?}\r\nhandled {} messages\r\n", i.thread_id, i.count);
    }

    // build and start cloneable actors.
    // Actors share the same address, actors don't share state and every one have an unique state of its own.
    let actor2 = CloneAbleActor::create(|| CloneAbleActor { state: 0 });

    let address2 = actor2.build().num(12).start_cloneable(|actor| {
        // This closure is used to add Clone bound requirement for our actor.
        actor
    });

    // send messages
    for _i in 0..1_000 {
        // Both shared_actor and cloneable_actor have the same type Message1
        // and we can specific call the one we want with a type path.
        let _: usize = address2.send(cloneable_actor::Message1).await.unwrap();
    }

    let state: usize = address2.send(cloneable_actor::Message1).await.unwrap();
    println!("State is: {}, should be smaller than 1000", state);
}
