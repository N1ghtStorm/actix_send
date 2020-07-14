use actix_send::prelude::*;

/*
    We can construct an actor that only does dynamic dispatch with no concrete message types.
    This gives you some flexibility at the cost of some performance.
    (Two more heap allocations for one dynamic dispatch compared to static dispatch message)
*/

#[tokio::main]
async fn main() {
    // create an actor instance. The create function would return our Actor struct.
    let builder = MyActor::builder(|| async { MyActor { state: 0 } });

    // build and start the actor(s).
    let address: Address<MyActor> = builder.start().await;

    // let the actor run a boxed future.
    let _ = address.run(|actor| Box::pin(actor.handle_message())).await;

    let mut var = String::from("outer variable");

    let state = address
        .run(move |actor| {
            // We can capture the (mut) reference of outer variable with move.
            let capture = &mut var;

            // We can not capture the outer variable in the boxed future for now.
            // So it has to be dereference / copy / clone and then move into the async block.
            let _async_capture = capture.clone();
            Box::pin(async move {
                let _async_capture = _async_capture;

                actor.state
            })
        })
        .await;

    println!("Current MyActor state is: {:#?}", state.unwrap());
}

/*
    Note that you DO NOT add no_static attribute arg when you have static typed messages for an actor.

    no_static is only used to tell the macro to ignore all Message/Result/Handler implementations.

    In other word all actors have the ability of running dynamic dispatched futures by default.
*/

#[actor(no_static)]
pub struct MyActor {
    pub state: usize,
}

impl MyActor {
    async fn handle_message(&mut self) {
        self.state += 1;
    }
}
