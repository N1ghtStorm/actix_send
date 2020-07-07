use std::time::Duration;

use actix_send::prelude::*;

use crate::my_actor::*;
use std::rc::Rc;

/*
    When enabling actix-runtime we have more freedom in our handle methods at the exchange of a
    single threaded runtime.
    (Arbiter support would added in the future for multi threads use case.).

    By default we don't enable actix runtime. Please run this example with:

    cargo run --example actix --no-default-features --features actix-runtime
*/

#[actix_rt::main]
async fn main() {
    let state = String::from("running");

    let actor = MyActor::create(|| MyActor { state });

    /*
       When build multiple actors they would all spawn on current thread where start method called.
       So we would have 4 actors share the same address working on the main thread.
       They would not block one another when processing messages.
    */
    let address = actor.build().num(4).start();

    let res = address.send(Message1).await.unwrap();

    println!("We got result for Message1\r\nResult is: {:?}\r\n", res);

    // register an interval future for actor with given duration.
    let handler = address
        .run_interval(Duration::from_secs(1), |_actor| {
            // Box the closure directly and wrap some async code in it.

            let rc = Rc::new(123);
            Box::pin(async move {
                let _ = actix_rt::time::delay_for(Duration::from_millis(1)).await;

                // the boxed future doesn't have to be Send. so we can use Rc across await point.
                println!("Rc is: {}", &rc);
            })
        })
        .await
        .unwrap();

    let mut interval = actix_rt::time::interval(Duration::from_secs(1));

    for i in 0..5 {
        if i == 3 {
            // cancel the interval future after 3 seconds.
            handler.cancel();
            println!("interval future stopped");
        }

        interval.tick().await;
    }
    println!("example finish successfully");
}

#[actor_mod]
pub mod my_actor {
    use super::*;
    use std::cell::RefCell;

    #[actor]
    pub struct MyActor {
        pub state: String,
    }

    #[message(result = "RefCell<u8>")]
    pub struct Message1;

    #[handler]
    impl Handler for MyActor {
        async fn handle(&mut self, _: Message1) -> RefCell<u8> {
            let mut cell = RefCell::new(123);

            let _ = actix_rt::time::delay_for(Duration::from_millis(1)).await;

            // the handle method doesn't have to be Send. so we can use RefCell across await point.
            println!("refcell is: {:?}", &mut cell);

            cell
        }
    }
}
