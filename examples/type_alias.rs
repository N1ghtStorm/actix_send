use actix_send::prelude::*;

use crate::actor1::Actor1;
use crate::actor2::Actor2;

/*
    Type alias can be used for a message type shared between multiple actors.
*/

// A message type we want to use on multiple actors.
pub struct Message;

#[actor_mod]
pub mod actor1 {
    use super::*;

    #[actor]
    pub struct Actor1;

    // use type alias to bind the message type
    #[message(result = "u8")]
    pub type Message1 = super::Message;

    #[handler]
    impl Handler for Actor1 {
        async fn handle(&mut self, _msg: Message1) -> u8 {
            1
        }
    }
}

#[actor_mod]
pub mod actor2 {
    use super::*;

    #[actor]
    pub struct Actor2;

    // use type alias to bind the message type
    #[message(result = "usize")]
    pub type Message1 = super::Message;

    #[handler]
    impl Handler for Actor2 {
        async fn handle(&mut self, _msg: Message1) -> usize {
            3
        }
    }
}

#[tokio::main]
async fn main() {
    let builder = Actor1::builder(|| async { Actor1 });
    let address1 = builder.start().await;

    let builder = Actor2::builder(|| async { Actor2 });
    let address2 = builder.start().await;

    let res = address1.send(Message).await.unwrap();
    assert_eq!(res, 1u8);

    let res = address2.send(Message).await.unwrap();
    assert_eq!(res, 3usize);
}
