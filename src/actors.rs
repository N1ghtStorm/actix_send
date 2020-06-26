use std::collections::VecDeque;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{
    // atomic::AtomicBool,
    Arc,
    Mutex,
};
// use std::task::{Context, Poll};
use std::time::Duration;

use async_channel::{unbounded, Receiver, SendError, Sender};
use futures_channel::oneshot::{channel, Canceled, Sender as OneshotSender};
use tokio::sync::Mutex as AsyncMutex;

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
        let (tx, rx) = unbounded::<ChannelMessage<A>>();

        let num = self.num;

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

        let (tx, rx) = unbounded::<ChannelMessage<AA>>();

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

fn spawn_loop<A>(mut actor: A, rx: Receiver<ChannelMessage<A>>)
where
    A: Actor + Handler + 'static,
    A::Message: Message + Send + 'static,
    <A::Message as Message>::Result: Send,
{
    let ctx: ActorContext<A> = ActorContext::new();

    runtime::spawn(async move {
        loop {
            // handle delayed messages first
            let opt = ctx.delayed_messages.lock().unwrap().pop_front();
            if let Some(msg) = opt {
                let _ = actor.handle(msg).await;
            }

            // handle interval futures

            // receive messages and handle.
            match rx.recv().await {
                // channel is gone we should quit now.
                // ToDo: we should graceful shutdown.
                Err(_) => break,
                Ok(msg) => match msg {
                    ChannelMessage::Instant(tx, msg) => {
                        let res = actor.handle(msg).await;
                        if let Some(tx) = tx {
                            let _ = tx.send(res);
                        }
                    }
                    ChannelMessage::Delayed(msg, dur) => {
                        let delayed = ctx.delayed_messages.clone();
                        // ToDo: We should add select for this future for canceling.
                        runtime::spawn(async move {
                            runtime::delay_for(dur).await;
                            delayed.lock().unwrap().push_back(msg);
                        })
                    }
                    // ChannelMessage::Interval(tx, interval, dur) => unimplemented!(),
                    _ => unimplemented!(),
                },
            }
        }
    });
}

struct ActorContext<A>
where
    A: Actor + Send,
    A::Message: Message + Send,
{
    // next_id: usize,
    // interval_futures: Arc<Mutex<Vec>>,
    // timed_handlers: Vec<Arc<AtomicBool>>,
    delayed_messages: Arc<Mutex<VecDeque<A::Message>>>,
}

impl<A> ActorContext<A>
where
    A: Actor + Send,
    A::Message: Message + Send,
{
    fn new() -> Self {
        Self {
            // next_id: 0,
            // interval_futures: Vec::new(),
            // timed_handlers: Vec::new(),
            delayed_messages: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

// struct IntervalFuture<A> {
//     id: usize,
//     func: Box<dyn Fn(&mut A) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>,
//     interval: Duration,
// }
//
// impl<A> Future for IntervalFuture<A> {
//     type Output = ();
//
//     fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
//         self.interval
//     }
// }

enum ChannelMessage<A>
where
    A: Actor,
    A::Message: Message,
{
    Instant(
        Option<OneshotSender<<A::Message as Message>::Result>>,
        A::Message,
    ),
    Delayed(A::Message, Duration),
    Interval(
        OneshotSender<IntervalHandler>,
        Box<dyn Fn(&mut A) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>,
        Duration,
    ),
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

#[derive(Clone)]
pub struct Address<A>
where
    A: Actor,
    A::Message: Message,
{
    tx: Sender<ChannelMessage<A>>,
    _a: PhantomData<A>,
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

    pub async fn register_interval<F, Fut>(
        &self,
        dur: Duration,
        f: F,
    ) -> Result<IntervalHandler, ActixSendError>
    where
        F: Fn(&mut A) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static,
    {
        let (tx, rx) = channel::<IntervalHandler>();

        let channel_message = ChannelMessage::Interval(tx, Box::new(f), dur);

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

pub struct IntervalHandler {}

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
