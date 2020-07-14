use std::sync::{atomic::AtomicUsize, Arc};

use actix_send::prelude::*;
use tokio::task::JoinError;

use crate::my_actor::*;

#[tokio::main]
async fn main() {
    let state2 = Arc::new(AtomicUsize::new(0));

    let builder = MyActor::builder(move || {
        let state2 = state2.clone();

        async { MyActor { state: 0, state2 } }
    });

    // We need to build 2 actors as one actor can handle only one message at a time.
    let address = builder.num(2).start().await;

    let addr = address.clone();
    let f1 = async move {
        let res = addr.send(Message).await;

        println!("We got result for Message last\r\nResult is: {:?}\r\n", res);
    };

    let f2 = async move {
        let res = address.send(Message2).await;
        println!(
            "We got result for Message2 first \r\nResult is: {:?}\r\n",
            res
        );
    };

    let _ = tokio::spawn(futures_util::future::join(f1, f2)).await;
}

#[actor_mod]
pub mod my_actor {
    use super::*;
    use std::time::Duration;

    #[actor]
    pub struct MyActor {
        pub state: usize,
        pub state2: Arc<AtomicUsize>,
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
            let res = tokio::task::spawn_blocking(move || {
                std::thread::sleep(Duration::from_secs(1));
                2
            })
            .await;

            res

            /*  Or some async after */
        }
    }

    #[handler]
    impl Handler for MyActor {
        fn handle(&self, _: Message2) -> usize {
            /*
                We marked Message2 as blocking in #[message] attribute
                So no async code allowed in handle method.
            */

            /*
              ***.  LIMITATION:

                   For now you can't access self state in this handle method.
                   As the whole method would be wrapped in a spawn_blocking function and send to
                   another thread.
            */

            // self.state += 1; // This line would cause failure for compile if you uncomment.

            std::thread::sleep(Duration::from_millis(100));

            1
        }
    }
}
