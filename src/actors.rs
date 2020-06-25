use std::marker::PhantomData;
use std::sync::Arc;

use async_channel::{unbounded, SendError, Sender};
use async_trait::async_trait;
use futures_channel::oneshot::{
    channel,
    Canceled,
    // Receiver,
    Sender as OneshotSender,
};
use tokio::sync::Mutex;

use crate::util::runtime;

pub struct Builder<A, M>
where
    A: Actor<M>,
    M: Message,
{
    actor: A,
    num: usize,
    _p: PhantomData<M>,
}

impl<A, M> Builder<A, M>
where
    A: Actor<M> + Handler<M> + 'static,
    M: Message + Send + 'static,
    M::Result: Send,
{
    /// Build multiple actors with the num passed.
    ///
    /// All the actors would steal work from a single async-channel(MPMC queue).
    pub fn num(mut self, num: usize) -> Self {
        assert!(num > 0, "The number of actors must be larger than 0");
        self.num = num;
        self
    }

    pub fn start(mut self) -> Address<M> {
        let (tx, rx) = unbounded::<(OneshotSender<M::Result>, M)>();

        let num = self.num;

        if num > 1 {
            let actor = Arc::new(Mutex::new(self.actor));
            for _ in 0..num {
                let actor = actor.clone();
                let rx = rx.clone();
                runtime::spawn(async move {
                    while let Ok((tx, msg)) = rx.recv().await {
                        let mut act = actor.lock().await;
                        let res = act.handle(msg).await;
                        let _ = tx.send(res);
                    }
                });
            }
        } else {
            runtime::spawn(async move {
                while let Ok((tx, msg)) = rx.recv().await {
                    let res = self.actor.handle(msg).await;
                    let _ = tx.send(res);
                }
            });
        }

        Address { tx }
    }
}

#[derive(Debug)]
pub enum ActixSendError {
    Canceled,
    Closed,
}

impl From<Canceled> for ActixSendError {
    fn from(_err: Canceled) -> Self {
        ActixSendError::Canceled
    }
}

impl<M> From<SendError<(OneshotSender<M::Result>, M)>> for ActixSendError
where
    M: Message,
{
    fn from(_err: SendError<(OneshotSender<M::Result>, M)>) -> Self {
        ActixSendError::Closed
    }
}

#[derive(Clone)]
pub struct Address<M>
where
    M: Message,
{
    tx: Sender<(OneshotSender<M::Result>, M)>,
}

impl<M> Address<M>
where
    M: Message,
{
    /// Type `R` is the same as Message's result type in `#[message]` macro
    ///
    /// Message will be returned in `ActixSendError::Closed(Message)` if the actor is already closed.
    pub async fn send<R>(&self, msg: impl Into<M>) -> Result<R, ActixSendError>
    where
        R: From<M::Result>,
    {
        let (tx, rx) = channel::<M::Result>();
        self.tx.send((tx, msg.into())).await?;

        let res = rx.await?;

        Ok(From::from(res))
    }
}

pub trait Actor<M>
where
    M: Message,
    Self: Sized + Send,
{
    fn build(self) -> Builder<Self, M> {
        Builder {
            actor: self,
            num: 1,
            _p: PhantomData,
        }
    }
}

pub trait Message: Send {
    type Result;
}

#[async_trait]
pub trait Handler<M>
where
    M: Message,
    Self: Actor<M>,
{
    async fn handle(&mut self, msg: M) -> M::Result;
}

#[cfg(feature = "tokio-runtime")]
#[cfg(not(feature = "async-std-runtime"))]
pub mod test_actor {
    use crate::prelude::*;

    #[actor_mod]
    pub mod my_actor {
        use crate::prelude::*;

        #[actor]
        pub struct MyActor {
            state1: String,
            state2: String,
        }

        #[message(result = "u8")]
        pub struct DummyMessage1 {
            pub from: String,
        }

        #[message(result = "u16")]
        pub struct DummyMessage2(pub u32, pub usize);

        #[handler]
        impl Handler for MyActor {
            // The msg and handle's return type must match former message macro's result type.
            async fn handle(&mut self, msg: DummyMessage1) -> u8 {
                assert_eq!("running1", self.state1);
                8
            }
        }

        #[handler]
        impl Handler for MyActor {
            async fn handle(&mut self, msg: DummyMessage2) -> u16 {
                assert_eq!("running2", self.state2);
                16
            }
        }
    }

    #[tokio::test]
    async fn test() {
        use super::test_actor::my_actor::*;
        let state1 = String::from("running1");
        let state2 = String::from("running2");
        let actor = MyActor::create(state1, state2);

        // build and start the actor(s).
        let address = actor.build().num(1).start();

        // construct a new message instance and convert it to a MessageObject
        let msg = DummyMessage1 {
            from: "a simple test".to_string(),
        };

        let msg2 = DummyMessage2(1, 2);

        // use address to send message object to actor and await on result.
        let res: u8 = address.send(msg).await.unwrap();

        let res2: u16 = address.send(msg2).await.unwrap();

        assert_eq!(res, 8);
        assert_eq!(res2, 16);
    }
}
