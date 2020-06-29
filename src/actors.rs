use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use async_channel::{unbounded, Sender};
use futures::channel::oneshot::channel;
use parking_lot::Mutex;

use crate::context::{spawn_loop, ChannelMessage};
use crate::error::ActixSendError;
use crate::interval::IntervalFutureSet;
use crate::util::{future_handle::FutureHandler, runtime};

pub struct Builder<A>
where
    A: Actor,
{
    actor: A,
    num: usize,
}

impl<A> Builder<A>
where
    A: Actor + Handler + Clone + 'static,
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
        let num = self.num;

        let (tx, rx) = unbounded::<ChannelMessage<A>>();

        let mut handlers = Vec::new();

        let interval_futures = IntervalFutureSet::new();

        if num > 1 {
            for _ in 0..num {
                let handler = spawn_loop(
                    self.actor.clone(),
                    tx.clone(),
                    rx.clone(),
                    interval_futures.clone(),
                );
                handlers.push(handler);
            }
        } else {
            let handler = spawn_loop(self.actor, tx.clone(), rx, interval_futures);
            handlers.push(handler);
        }

        Address::new(tx, handlers)
    }

    fn check_num(num: usize, target: usize) {
        assert!(
            num > target,
            "The number of actors must be larger than {}",
            target
        );
    }
}

pub struct Address<A>
where
    A: Actor + 'static,
    A::Message: Message,
    <A::Message as Message>::Result: Send,
{
    tx: Sender<ChannelMessage<A>>,
    handlers: Arc<Mutex<Vec<FutureHandler<A>>>>,
    _a: PhantomData<A>,
}

impl<A> Address<A>
where
    A: Actor + 'static,
    A::Message: Message,
    <A::Message as Message>::Result: Send,
{
    fn new(tx: Sender<ChannelMessage<A>>, handlers: Vec<FutureHandler<A>>) -> Self {
        Self {
            tx,
            handlers: Arc::new(Mutex::new(handlers)),
            _a: PhantomData,
        }
    }
}

impl<A> Clone for Address<A>
where
    A: Actor + 'static,
    A::Message: Message,
    <A::Message as Message>::Result: Send,
{
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            handlers: self.handlers.clone(),
            _a: PhantomData,
        }
    }
}

impl<A> Drop for Address<A>
where
    A: Actor + 'static,
    A::Message: Message,
    <A::Message as Message>::Result: Send,
{
    fn drop(&mut self) {
        if Arc::strong_count(&self.handlers) == 1 {
            for handler in self.handlers.lock().iter() {
                handler.cancel();
            }
        }
    }
}

impl<A> Address<A>
where
    A: Actor + 'static,
    A::Message: Message,
    <A::Message as Message>::Result: Send,
{
    /// Type `R` is the same as Message's result type in `#[message]` macro
    ///
    /// Message will be returned in `ActixSendError::Closed(Message)` if the actor is already closed.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn send<M>(
        &self,
        msg: M,
    ) -> Result<<M as MapResult<<A::Message as Message>::Result>>::Output, ActixSendError>
    where
        M: Into<A::Message> + MapResult<<A::Message as Message>::Result>,
    {
        let (tx, rx) = channel::<<A::Message as Message>::Result>();

        let channel_message = ChannelMessage::Instant(Some(tx), msg.into());

        self.tx.send(channel_message).await?;

        let res = rx.await?;

        Ok(M::map(res))
    }

    /// Send a message to actor and ignore the result.
    pub fn do_send(&self, msg: impl Into<A::Message> + Send + 'static) {
        let msg = ChannelMessage::Instant(None, msg.into());
        self._do_send(msg);
    }

    /// run a message after a certain amount of delay.
    pub fn run_later(&self, msg: impl Into<A::Message> + Send + 'static, delay: Duration) {
        let msg = ChannelMessage::Delayed(msg.into(), delay);
        self._do_send(msg);
    }

    /// register an interval future for actor. An actor can have multiple interval futures registered.
    ///
    /// a `FutureHandler` would return that can be used to cancel it.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn run_interval<F, Fut>(
        &self,
        dur: Duration,
        f: F,
    ) -> Result<FutureHandler<A>, ActixSendError>
    where
        F: Fn(A) -> Fut + Send + 'static,
        Fut: Future<Output = A> + Send + 'static,
    {
        let (tx, rx) = channel::<FutureHandler<A>>();

        let object = crate::interval::IntervalFutureContainer(f, PhantomData, PhantomData).pack();

        let channel_message = ChannelMessage::Interval(tx, object, dur);

        self.tx.send(channel_message).await?;

        Ok(rx.await?)
    }

    fn _do_send(&self, msg: ChannelMessage<A>) {
        let this = self.tx.clone();
        runtime::spawn(async move {
            let _ = this.send(msg).await;
        });
    }
}

// a helper trait for map result of original messages.
// M here is auto generated ActorResult from #[actor_mod] macro.
pub trait MapResult<M>: Sized {
    type Output;
    fn map(msg: M) -> Self::Output;
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

// ToDo: Do we still need a message trait?. Message is already an associate type of Actor we can move Result type to Actor as well.
pub trait Message: Send {
    type Result;
}

#[async_trait::async_trait]
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
        let address: Address<TestActor> = actor.build().num(1).start();

        // construct a new message instance and convert it to a MessageObject
        let msg = DummyMessage1 {
            from: "a simple test".to_string(),
        };

        let msg2 = DummyMessage2(1, 2);

        // use address to send message object to actor and await on result.
        let res = address.send(msg).await.unwrap();

        let res2 = address.send(msg2).await.unwrap();

        assert_eq!(res, 8);
        assert_eq!(res2, 16);
    }
}
