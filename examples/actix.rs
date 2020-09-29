/*
    When enabling actix-runtime we have more freedom in our handle methods at the exchange of a
    single threaded runtime.

    The actor's address can still be sent between threads and actor can handle messages from other
    threads.

    By default we don't enable actix runtime. Please run this example with:

    cargo run --example actix --no-default-features --features actix-runtime
*/
#[cfg(feature = "actix-runtime")]
#[actix_rt::main]
async fn main() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use actix::Arbiter;
    use actix_send::prelude::*;

    #[actor]
    pub struct MyActor {
        pub state: String,
    }

    pub struct Message1;

    #[handler_v2]
    impl Handler for MyActor {
        async fn handle(&mut self, _: Message1) -> RefCell<u8> {
            let mut cell = RefCell::new(123);

            let _ = actix_rt::time::delay_for(Duration::from_millis(1)).await;

            // the handle method doesn't have to be Send. so we can use RefCell across await point.
            println!("refcell is: {:?}", &mut cell);

            cell
        }
    }

    let builder = MyActor::builder(|| async {
        let state = String::from("running");
        MyActor { state }
    });

    /*
       When build multiple actors they would all spawn on current thread where start method called.
       So we would have 4 actors share the same address working on the main thread.
       They would not block one another when processing messages.
    */
    let address = builder.num(4).start().await;

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

    drop(address);

    let builder = MyActor::builder(|| async {
        let state = String::from("running2");
        MyActor { state }
    });

    /*
        We can utilize arbiters and spawn our actors on a thread other than the current one.
    */

    // build a set of arbiters.
    let arbiters = (0..6).map(|_| Arbiter::new()).collect::<Vec<Arbiter>>();

    /*
        Start multiple actors on the given arbiters. The actors would try to spawn on them evenly.
        Note that we pass a slice of the Vec<Arbiter> to the arg so you can pass partial slice for
        a more precise control.
    */

    let address = builder
        .num(12)
        .start_with_arbiters(&arbiters[2..5], None)
        .await;

    let _ = actix_rt::time::delay_for(Duration::from_secs(1)).await;

    println!(
        "current active actor count is: {}",
        address.current_active()
    );

    println!("example finish successfully");
}

#[cfg(not(feature = "actix-runtime"))]
fn main() {}
