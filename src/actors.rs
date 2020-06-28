use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use async_channel::{unbounded, Receiver, SendError, Sender};
use futures::channel::oneshot::{channel, Canceled, Sender as OneshotSender};
use parking_lot::Mutex;
use tokio::sync::Mutex as AsyncMutex;

use crate::context::ActorContext;
use crate::interval::IntervalFuture;
use crate::util::{
    future_handle::{spawn_cancelable, FutureHandler},
    runtime,
};

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
        let (tx, rx) = unbounded::<ChannelMessage<A>>();

        let num = self.num;

        let mut handlers = Vec::new();

        if num > 1 {
            let actor = Arc::new(AsyncMutex::new(self.actor));
            for _ in 0..num {
                let actor = actor.clone();
                let rx = rx.clone();
                runtime::spawn(async move {
                    while let Ok(msg) = rx.recv().await {
                        if let ChannelMessage::Instant(tx, msg) = msg {
                            let mut act = actor.lock().await;
                            let res = act.handle(msg).await;
                            if let Some(tx) = tx {
                                let _ = tx.send(res);
                            }
                        }
                    }
                });
            }
        } else {
            let handler = spawn_loop(self.actor, tx.clone(), rx);
            handlers.push(handler);
        }

        Address::new(tx, handlers)
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

        let (tx, rx) = unbounded::<ChannelMessage<AA>>();

        let mut handlers = Vec::new();

        for _ in 0..num {
            let handler = spawn_loop(actor.clone(), tx.clone(), rx.clone());
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

// the gut of the event loop of an actor.
fn spawn_loop<A>(
    actor: A,
    tx: Sender<ChannelMessage<A>>,
    rx: Receiver<ChannelMessage<A>>,
) -> FutureHandler
where
    A: Actor + Handler + 'static,
    A::Message: Message + Send + 'static,
    <A::Message as Message>::Result: Send,
{
    let mut ctx: ActorContext<A> = ActorContext::new(tx, actor);

    let fut = async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                ChannelMessage::Instant(tx, msg) => {
                    let res = ctx.actor.as_mut().unwrap().handle(msg).await;
                    if let Some(tx) = tx {
                        let _ = tx.send(res);
                    }
                }
                ChannelMessage::Delayed(msg, dur) => {
                    let tx = ctx.tx.clone();
                    let delayed_handler = spawn_cancelable(Box::pin(async move {
                        runtime::delay_for(dur).await;
                        let _ = tx.send(ChannelMessage::Instant(None, msg)).await;
                    }));
                    ctx.delayed_handlers.push(delayed_handler);
                }
                ChannelMessage::IntervalFuture(idx) => {
                    if let Some(fut) = ctx.interval_futures.get_mut(idx) {
                        let act = ctx.actor.take().unwrap_or_else(|| panic!("Actor is gone"));
                        let act = fut.handle(act).await;
                        ctx.actor = Some(act);
                    }
                }
                ChannelMessage::Interval(tx, interval_future, dur) => {
                    // push interval future to context and get it's index
                    // ToDo: For now we don't have a way to remove the interval future if it's canceled using handler.
                    ctx.interval_futures.push(interval_future);
                    let index = ctx.interval_futures.len() - 1;

                    // construct the interval future
                    let mut interval = runtime::interval(dur);
                    let ctx_tx = ctx.tx.clone();
                    let interval_loop = Box::pin(async move {
                        loop {
                            let _ = interval.tick().await;
                            let _ = ctx_tx.send(ChannelMessage::IntervalFuture(index)).await;
                        }
                    });

                    // spawn a cancelable future and use the handler to execute the cancellation.
                    let handler = spawn_cancelable(interval_loop);

                    let _ = tx.send(handler);
                }
            }
        }
    };

    spawn_cancelable(Box::pin(fut))
}

pub(crate) enum ChannelMessage<A>
where
    A: Actor,
    A::Message: Message,
{
    Instant(
        Option<OneshotSender<<A::Message as Message>::Result>>,
        A::Message,
    ),
    Delayed(A::Message, Duration),
    Interval(OneshotSender<FutureHandler>, IntervalFuture<A>, Duration),
    IntervalFuture(usize),
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

impl<A> From<SendError<ChannelMessage<A>>> for ActixSendError
where
    A: Actor,
    A::Message: Message,
{
    fn from(_err: SendError<ChannelMessage<A>>) -> Self {
        ActixSendError::Closed
    }
}

pub struct Address<A>
where
    A: Actor + 'static,
    A::Message: Message,
    <A::Message as Message>::Result: Send,
{
    tx: Sender<ChannelMessage<A>>,
    handlers: Arc<Mutex<Vec<FutureHandler>>>,
    _a: PhantomData<A>,
}

impl<A> Address<A>
where
    A: Actor + 'static,
    A::Message: Message,
    <A::Message as Message>::Result: Send,
{
    fn new(tx: Sender<ChannelMessage<A>>, handlers: Vec<FutureHandler>) -> Self {
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
    pub async fn send<R>(&self, msg: impl Into<A::Message>) -> Result<R, ActixSendError>
    where
        R: From<<A::Message as Message>::Result>,
    {
        let (tx, rx) = channel::<<A::Message as Message>::Result>();

        let channel_message = ChannelMessage::Instant(Some(tx), msg.into());

        self.tx.send(channel_message).await?;

        let res = rx.await?;

        Ok(From::from(res))
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
    /// a `IntervalHandler` would return that can be used to cancel it.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn run_interval<F, Fut>(
        &self,
        dur: Duration,
        f: F,
    ) -> Result<FutureHandler, ActixSendError>
    where
        F: Fn(A) -> Fut + Send + 'static,
        Fut: Future<Output = A> + Send + 'static,
    {
        let (tx, rx) = channel::<FutureHandler>();

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
        let res: u8 = address.send(msg).await.unwrap();

        let res2: u16 = address.send(msg2).await.unwrap();

        assert_eq!(res, 8);
        assert_eq!(res2, 16);
    }
}
