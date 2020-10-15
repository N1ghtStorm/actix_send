use std::time::Duration;

use actix_send::prelude::*;

use crate::my_actor::*;
use crate::my_actor2::*;

#[tokio::main]
async fn main() {
    let builder1 = MyActor::builder(|| async { MyActor });

    let builder2 = MyActor2::builder(|| async { MyActor2 });

    let address1 = builder1
        // We can handle delayed message before shutdown
        .handle_delayed_on_shutdown()
        .start()
        .await;

    let address2 = builder2.start().await;

    for _ in 0..6 {
        // send messages after 10 seconds to actors.
        let _ = address1.send_later(Message1, Duration::from_secs(10)).await;
        let _ = address2.send_later(Message2, Duration::from_secs(10)).await;

        // run futures after 10 seconds from actors.
        let _ = address1
            .run_later(Duration::from_secs(10), |_actor| {
                Box::pin(async move {
                    println!("We ran a future for actor1.\r\n");
                })
            })
            .await;
        let _ = address2
            .run_later(Duration::from_secs(10), |_actor| {
                Box::pin(async move {
                    println!("We ran a future for actor2.\r\n");
                })
            })
            .await;
    }

    // We wait a little bit before we drop the address so that all delayed requests are registered.
    let _ = tokio::time::sleep(Duration::from_secs(1)).await;

    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);

    /*
       addresses are dropped here and our actors would shut down.

       Since we didn't enable handle_delayed_on_shutdown for actor2.
       All the delay messages and boxed futures didn't met the deadline for actor2 are dropped on
       actor shutdown.
    */

    drop(address1);
    drop(address2);
    let _ = tokio::time::sleep(Duration::from_secs(1)).await;
}

#[actor_mod]
pub mod my_actor {
    use super::*;

    #[actor]
    pub struct MyActor;

    #[message(result = "()")]
    pub struct Message1;

    #[handler]
    impl Handler for MyActor {
        async fn handle(&mut self, _: Message1) {
            println!("We got a Message1.\r\n");
        }
    }
}

#[actor_mod]
pub mod my_actor2 {
    use super::*;

    #[actor]
    pub struct MyActor2;

    #[message(result = "()")]
    pub struct Message2;

    #[handler]
    impl Handler for MyActor2 {
        async fn handle(&mut self, _: Message2) {
            println!("We got a Message2.\r\n");
        }
    }
}
