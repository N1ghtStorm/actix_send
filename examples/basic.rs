use actix_send::prelude::*;
use async_trait::async_trait;

// construct a new actor.
#[actor]
struct MyActor {
    state: String,
}

// construct a new message with a result type.
#[message(result = "Option<MyResult>")]
#[derive(Debug)]
struct MyMessage {
    from: String,
    content: String,
}

// dummy result type for MyMessage
#[derive(Debug)]
struct MyResult(u32);

// impl MyMessage handler for MyActor
// handler attribute must be placed above async_trait.
#[handler]
#[async_trait]
impl Handler for MyActor {
    // The msg and handle's return type must match former message macro's result type.
    async fn handle(&mut self, msg: MyMessage) -> Option<MyResult> {
        println!(
            "Actor state is: {}\r\n\r\nGot Message from: {}\r\n\r\nContent: {}",
            self.state,
            msg.from.as_str(),
            msg.content.as_str()
        );

        Some(MyResult(123))
    }
}

#[tokio::main]
async fn main() {
    // create an actor instance. The args passed to create function are in the same order and type of your Actor's struct fields.
    let state = String::from("running");
    let actor = MyActor::create(state);

    // build and start the actor(s).
    let address = actor.build().num(1).start();

    // use address to send message to actor and await on result.
    let result = address
        .send(MyMessage {
            from: "actix-send".to_string(),
            content: "a simple test".to_string(),
        })
        .await;

    println!("We got result for message: {:?}", result);
}
