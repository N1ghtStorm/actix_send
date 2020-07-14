use std::pin::Pin;
use std::task::{Context, Poll};

use actix_send::prelude::*;
use futures_util::stream::{Stream, StreamExt};

use crate::my_actor::*;

#[tokio::main]
async fn main() {
    let builder = MyActor::builder(|| async { MyActor });
    let address = builder.start().await;

    // create a mock stream
    let stream = MockStream { offset: 0 };

    // by sending the stream to actor we can handle every stream item as a message
    // and return the result as a new stream

    // since our mock stream produces String so we map the item from string to Message1
    let mut new_stream = address.send_stream(stream.map(|from| Message1 { from }));

    let future1 = async move {
        // result here would be Result<MessageResult, ActixSendError>
        while let Some(res) = new_stream.next().await {
            assert_eq!("message from stream", res.unwrap().as_str())
        }
    };

    // We can send the same message type as the stream through normal message at the same time.
    // We won't get our result crossed.
    let addr = address.clone();
    let future2 = async move {
        for i in 0..5 {
            let from = format!("message from sender {}", i);

            let res = addr.send(Message1 { from: from.clone() }).await.unwrap();
            assert_eq!(from, res);
        }
    };

    futures_util::future::join(future1, future2).await;
}

#[actor_mod]
pub mod my_actor {
    use super::*;

    #[actor]
    pub struct MyActor;

    #[message(result = "String")]
    pub struct Message1 {
        pub from: String,
    }

    #[handler]
    impl Handler for MyActor {
        async fn handle(&mut self, msg: Message1) -> String {
            // We just echo back the from field of incoming message.
            msg.from
        }
    }
}

// a mock stream that would produce String.
struct MockStream {
    offset: usize,
}

impl Stream for MockStream {
    type Item = String;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if this.offset == 5 {
            Poll::Ready(None)
        } else {
            this.offset += 1;
            Poll::Ready(Some(String::from("message from stream")))
        }
    }
}
