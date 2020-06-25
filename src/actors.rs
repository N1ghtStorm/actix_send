use std::marker::PhantomData;
use std::sync::Arc;

use async_channel::{unbounded, Receiver, SendError, Sender};
use async_trait::async_trait;
use futures_channel::oneshot::{channel, Canceled, Sender as OneshotSender};
use tokio::sync::Mutex;

use crate::util::runtime;

pub struct Builder<A>
where
    A: Actor,
{
    actor: A,
    num: usize,
}

impl<A> Builder<A>
where
    A: Actor + Handler + 'static,
    A::Message: Message + Send + 'static,
    <A::Message as Message>::Result: Send,
{
    /// Build multiple actors with the num passed.
    ///
    /// All the actors would steal work from a single `async-channel`.
    pub fn num(mut self, num: usize) -> Self {
        Self::check_num(num, 0);
        self.num = num;
        self
    }

    pub fn start(self) -> Address<A> {
        let (tx, rx) = unbounded::<ChannelSender<A::Message>>();

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
                        if let Some(tx) = tx {
                            let _ = tx.send(res);
                        }
                    }
                });
            }
        } else {
            spawn_loop(self.actor, rx);
        }

        Address {
            tx,
            _a: PhantomData,
        }
    }

    /// Start cloneable actors.
    ///
    /// *. Actors do not share state as we need `&mut Self` with every actor.
    pub fn start_cloneable<F, AA>(self, f: F) -> Address<AA>
    where
        F: FnOnce(A) -> AA,
        AA: Actor + Handler + Clone + 'static,
        AA::Message: Message + Send + 'static,
        <AA::Message as Message>::Result: Send,
    {
        let num = self.num;

        Self::check_num(num, 1);

        let actor = f(self.actor);

        let (tx, rx) = unbounded::<ChannelSender<AA::Message>>();

        for _ in 0..num {
            let actor = actor.clone();
            let rx = rx.clone();
            spawn_loop(actor, rx);
        }

        Address {
            tx,
            _a: PhantomData,
        }
    }

    fn check_num(num: usize, target: usize) {
        assert!(
            num > target,
            "The number of actors must be larger than {}",
            target
        );
    }
}

fn spawn_loop<A: Actor>(mut actor: A, rx: Receiver<ChannelSender<A::Message>>)
where
    A: Actor + Handler + 'static,
    A::Message: Message + Send + 'static,
    <A::Message as Message>::Result: Send,
{
    runtime::spawn(async move {
        while let Ok((tx, msg)) = rx.recv().await {
            let res = actor.handle(msg).await;
            if let Some(tx) = tx {
                let _ = tx.send(res);
            }
        }
    });
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

impl<M> From<SendError<ChannelSender<M>>> for ActixSendError
where
    M: Message,
{
    fn from(_err: SendError<ChannelSender<M>>) -> Self {
        ActixSendError::Closed
    }
}

type ChannelSender<M> = (Option<OneshotSender<<M as Message>::Result>>, M);

#[derive(Clone)]
pub struct Address<A>
where
    A: Actor,
    A::Message: Message,
{
    tx: Sender<ChannelSender<A::Message>>,
    _a: PhantomData<A>,
}

impl<A> Address<A>
where
    A: Actor,
    A::Message: Message + Send + 'static,
    <A::Message as Message>::Result: Send,
{
    /// Type `R` is the same as Message's result type in `#[message]` macro
    ///
    /// Message will be returned in `ActixSendError::Closed(Message)` if the actor is already closed.
    pub async fn send<R>(&self, msg: impl Into<A::Message>) -> Result<R, ActixSendError>
    where
        R: From<<A::Message as Message>::Result>,
    {
        let (tx, rx) = channel::<<A::Message as Message>::Result>();
        self.tx.send((Some(tx), msg.into())).await?;

        let res = rx.await?;

        Ok(From::from(res))
    }

    /// Send a message to actor and ignore the result.
    pub fn do_send(&self, msg: impl Into<A::Message> + Send + 'static) {
        let this = self.tx.clone();

        runtime::spawn(async move {
            let _ = this.send((None, msg.into())).await;
        });
    }
}

pub trait Actor
where
    Self: Sized + Send,
{
    type Message;

    fn build(self) -> Builder<Self> {
        Builder {
            actor: self,
            num: 1,
        }
    }
}

// ToDo: Do we still need a message trait?. Since Message is already an associate type of Actor we can move Result type to Actor as well.
pub trait Message: Send {
    type Result;
}

#[async_trait]
pub trait Handler
where
    Self: Actor,
    <Self as Actor>::Message: Message,
{
    async fn handle(&mut self, msg: Self::Message) -> <Self::Message as Message>::Result;
}

#[cfg(feature = "tokio-runtime")]
#[cfg(not(feature = "async-std-runtime"))]
pub mod test_actor {
    use crate::prelude::*;

    #[actor_mod]
    pub mod my_actor {
        use crate::prelude::*;

        #[actor]
        pub struct TestActor {
            pub state1: String,
            pub state2: String,
        }

        #[message(result = "u8")]
        pub struct DummyMessage1 {
            pub from: String,
        }

        #[message(result = "u16")]
        pub struct DummyMessage2(pub u32, pub usize);

        #[handler]
        impl Handler for TestActor {
            // The msg and handle's return type must match former message macro's result type.
            async fn handle(&mut self, msg: DummyMessage1) -> u8 {
                assert_eq!("running1", self.state1);
                8
            }
        }

        #[handler]
        impl Handler for TestActor {
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
        let actor = TestActor::create(|| TestActor { state1, state2 });

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
