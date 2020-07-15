use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use actix_send::prelude::*;

#[actor]
pub struct MyActor {
    state: Arc<AtomicUsize>,
}

// The broadcast message must be able to clone itself as we want to send it to multiple actors.
#[derive(Clone)]
pub struct Message;

#[handler_v2]
impl MyActor {
    async fn handle_msg1(&mut self, _: Message) -> usize {
        self.state.fetch_add(1, Ordering::Relaxed)
    }
}

impl MyActor {
    async fn state(&self) -> usize {
        self.state.load(Ordering::Relaxed)
    }
}

#[tokio::main]
async fn main() {
    let state = Arc::new(AtomicUsize::new(0));

    let builder = MyActor::builder(move || {
        let state = state.clone();
        async { MyActor { state } }
    });

    let address: Address<MyActor> = builder.num(8).start().await;

    /*
       broadcast a message to every actor of this address.
       The broadcast will return a vector of result regardless individual actor succeed or failed.
    */
    let res = address
        .broadcast(Message)
        .await
        .into_iter()
        .fold(0usize, |i, c| i + c.unwrap());

    assert_eq!(28, res);

    let state = address.run(|actor| Box::pin(actor.state())).await.unwrap();

    assert_eq!(8, state);
}
