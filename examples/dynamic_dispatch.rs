use actix_send::prelude::*;

/*
    We can construct an actor that only does dynamic dispatch with no concrete message types.
    This gives you some flexibility at the cost of some performance.
    (Two more heap allocations for one dynamic dispatch compared to static dispatch message)
*/

#[tokio::main]
async fn main() {
    // create an actor instance. The create function would return our Actor struct.
    let actor = MyActor::create(|| MyActor { state: 0 });

    // build and start the actor(s).
    let address: Address<MyActor> = actor.build().start();

    // let the actor run a boxed future.
    let _ = address
        .run(|actor| {
            Box::pin(async move {
                actor.state += 1;
            })
        })
        .await;

    let state = address
        .run(|actor| Box::pin(async move { actor.state }))
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