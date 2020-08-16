/*
    When enabling actix-runtime-local the actor is strictly run on single thread.

    It means the actor's address can not be send to other threads and no message from other threads
    can be handled.

    By default we don't enable actix runtime. Please run this example with:

    cargo run --example actix --no-default-features --features actix-runtime-local
*/

fn main() {
    #[cfg(feature = "actix-runtime-local")]
    fn _main() {
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::time::Duration;

        use actix_send::prelude::*;

        //use no_send to notify the macro we want a !Send actor.
        #[actor(no_send)]
        pub struct MyActor {
            // Since the actor is runs on single thread. It can have state with !Send bound
            pub state: Rc<String>,
        }

        pub struct Message1;

        //use no_send to notify the macro the handler method can be !Send futures.
        #[handler_v2(no_send)]
        impl MyActor {
            async fn handle_msg1(&mut self, _: Message1) -> RefCell<u8> {
                let mut cell = RefCell::new(123);

                let _ = actix_rt::time::delay_for(Duration::from_millis(1)).await;

                println!("refcell is: {:?}", &mut cell);

                cell
            }
        }

        actix_rt::System::new("actix-test").block_on(async {
            let builder = MyActor::builder(|| async {
                let state = Rc::new(String::from("running"));
                MyActor { state }
            });

            let address = builder.start().await;

            let res = address.send(Message1).await.unwrap();

            println!("We got result for Message1\r\nResult is: {:?}\r\n", res);

            let handler = address
                .run_interval(Duration::from_secs(1), |_actor| {
                    let rc = Rc::new(123);
                    Box::pin(async move {
                        let _ = actix_rt::time::delay_for(Duration::from_millis(1)).await;
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
        })
    }

    #[cfg(feature = "actix-runtime-local")]
    _main();
}
