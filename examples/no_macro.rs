use actix_send::prelude::*;

/*
    A hand written implementation of actor
    This is basically the code macro would generate.
*/

// actor impl with Clone(For multiple instances construction only )
#[derive(Clone)]
pub struct MyActor {
    pub state: String,
}

// impl Actor trait.
impl Actor for MyActor {
    type Message = MyActorMessage;
    type Result = MyActorResult;
}

// message types
pub struct Message1(u8);
pub struct Message2(u16);

// construct enum to contain all message types for <MyActor as Actor>::Message
pub enum MyActorMessage {
    Message1(Message1),
    Message2(Message2),
}

// impl From trait for auto converting message types to enum
impl From<Message1> for MyActorMessage {
    fn from(msg: Message1) -> Self {
        MyActorMessage::Message1(msg)
    }
}

impl From<Message2> for MyActorMessage {
    fn from(msg: Message2) -> Self {
        MyActorMessage::Message2(msg)
    }
}

// construct enum to contain all message result types for <MyActor as Actor>::Result
pub enum MyActorResult {
    Message1Res(u32),
    Message2Res(u64),
}

// impl MapResult trait for auto converting enum result to message's result type.
impl MapResult<MyActorResult> for Message1 {
    type Output = u32;

    fn map(msg: MyActorResult) -> Self::Output {
        match msg {
            MyActorResult::Message1Res(res) => res,
            _ => unreachable!(),
        }
    }
}

impl MapResult<MyActorResult> for Message2 {
    type Output = u64;

    fn map(msg: MyActorResult) -> Self::Output {
        match msg {
            MyActorResult::Message2Res(res) => res,
            _ => unreachable!(),
        }
    }
}

// impl handler and async_trait attribute is required.
// return type of every arm of enum must be the same as <Message as MapResult<MyActorResult>>::Output
#[async_trait]
impl Handler for MyActor {
    async fn handle(&mut self, msg: <MyActor as Actor>::Message) -> <MyActor as Actor>::Result {
        match msg {
            MyActorMessage::Message1(msg) => MyActorResult::Message1Res(msg.0 as u32),
            MyActorMessage::Message2(msg) => MyActorResult::Message2Res(msg.0 as u64),
        }
    }
}

#[tokio::main]
async fn main() {
    let state = String::from("running");

    let actor = MyActor::create(|| MyActor { state });

    let address = actor.build().start();

    let res = address.send(Message1(8)).await.unwrap();
    assert_eq!(res, 8u32);

    let res = address.send(Message2(16)).await.unwrap();
    assert_eq!(res, 16u64);
}
