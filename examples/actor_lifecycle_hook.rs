use actix::clock::Duration;
use actix_send::prelude::*;

// Actor type
// Note we can't use no_static attribute here. Because we have no way figuring out if we have a
// #[handler_v2] macro later so we can't implement a dummy one for it.
#[actor]
pub struct MyActor;

// message types
pub struct Message;

// handler implement
#[handler_v2]
impl MyActor {
    // #[on_start] attribute indicate this function is called when actor starting.
    // this happens for every actor instance if you have multiple actors for one address.
    #[on_start]
    fn on_start(&mut self) {
        println!("this happens before actor is started");
    }

    // #[on_stop] attribute indicate this function is called before actor shutdown.
    // this happens for every actor instance if you have multiple actors for one address.
    #[on_stop]
    fn on_off_on_off_maybe_stop(&mut self) {
        // Just like other methods. The actual name of method is not important at all.
        // It's the #[on_stop] that matters.

        println!("this happens before actor is closed");
    }

    async fn handle_msg1(&mut self, _msg1: Message) {
        println!("we got a message");
    }
}

#[tokio::main]
async fn main() {
    let builder = MyActor::builder(|| async { MyActor });

    let address: Address<MyActor> = builder.start().await;

    address.send(Message).await.unwrap();

    drop(address);

    tokio::time::delay_for(Duration::from_secs(1)).await;

    println!("example finished successfully");
}
