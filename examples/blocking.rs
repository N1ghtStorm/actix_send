use actix_send::prelude::*;
use tokio::task::JoinError;

use crate::my_actor::*;

#[tokio::main(threaded_scheduler, core_threads = 12)]
async fn main() {
    let actor = MyActor::create(|| MyActor { state: 0 });

    let address = actor.build().start();

    let (tx, rx) = futures::channel::oneshot::channel::<()>();

    let addr = address.clone();
    tokio::spawn(async move {
        let res = addr.send(Message).await;

        println!("We got result for Message last\r\nResult is: {:?}\r\n", res);

        let _ = tx.send(());
    });

    // We are not blocked by the task;

    let res = address.send(Message2).await;

    println!(
        "We got result for Message2 first \r\nResult is: {:?}\r\n",
        res
    );

    let _ = rx.await;
}

#[actor_mod]
pub mod my_actor {
    use super::*;

    #[actor]
    pub struct MyActor {
        pub state: usize,
    }

    /*
       There are two ways to call blocking code with actix_send:
       1. Call runtime specific blocking features directly in handle method.

       2. utilize message attribute with #[message(result = T, blocking)]

       The first way is preferable as it would give you ability to mix blocking code with async
       code in handle method.
       The second way would result in a blocking code only handle method.
    */

    #[message(result = "Result<usize, JoinError>")]
    pub struct Message;

    #[message(result = "usize", blocking)]
    pub struct Message2;

    #[handler]
    impl Handler for MyActor {
        async fn handle(&mut self, _: Message) -> Result<usize, JoinError> {
            /*  You ca do some async computation first  */

            // just call blocking feature of your runtime directly.
            tokio::task::spawn_blocking(move || {
                let mut i = 0usize;
                for _ in 0..2_000_000 {
                    i += 1;
                }
                i
            })
            .await

            /*  Or some async after */
        }
    }

    #[handler]
    impl Handler for MyActor {
        fn handle(&mut self, _: Message2) -> usize {
            /*
                We marked Message2 as blocking in #[message] attribute
                So no async code allowed in handle method.
            */

            println!("{}", self.state);
            let mut i = 0usize;
            for _ in 0..1_000_000 {
                i += 1;
            }
            i
        }
    }
}
