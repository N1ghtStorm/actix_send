use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use futures_util::stream::{FuturesUnordered, Stream, StreamExt};

use crate::actor::{Actor, ActorState};
use crate::context::{
    ActorContextState, ContextMessage, DelayedMessage, InstantMessage, IntervalMessage,
};
use crate::error::ActixSendError;
use crate::object::AnyObjectContainer;
use crate::sender::{GroupSender, Sender, WeakGroupSender, WeakSender};
use crate::stream::{ActorSkipStream, ActorStream};
use crate::subscribe::Subscribe;
use crate::util::{
    channel::oneshot_channel, future_handle::FutureHandler, runtime, smart_pointer::RefCounter,
};

// A channel sender for communicating with actor(s).
pub struct Address<A>
where
    A: Actor + 'static,
{
    strong_count: RefCounter<AtomicUsize>,
    tx: Sender<ContextMessage<A>>,
    tx_subs: Option<GroupSender<A>>,
    subs: Option<Subscribe>,
    state: ActorState<A>,
}

impl<A> Clone for Address<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        self.strong_count.fetch_add(1, Ordering::Relaxed);
        Self {
            strong_count: self.strong_count.clone(),
            tx: self.tx.clone(),
            tx_subs: self.tx_subs.clone(),
            subs: self.subs.clone(),
            state: self.state.clone(),
        }
    }
}

impl<A> Drop for Address<A>
where
    A: Actor,
{
    fn drop(&mut self) {
        if self.strong_count.fetch_sub(1, Ordering::Acquire) == 0 {
            self.state.shutdown();
        }
    }
}

impl<A> Address<A>
where
    A: Actor,
{
    /// Downgrade to a Weak version of address which can be upgraded to a Address later.
    pub fn downgrade(&self) -> WeakAddress<A> {
        WeakAddress {
            strong_count: self.strong_count.clone(),
            tx: self.tx.downgrade(),
            tx_subs: self.tx_subs.as_ref().map(|sub| sub.downgrade()),
            state: self.state.clone(),
        }
    }

    /// The number of currently active actors for the given address.
    pub fn current_active(&self) -> usize {
        self.state.current_active()
    }

    pub(crate) fn new(
        tx: Sender<ContextMessage<A>>,
        tx_subs: GroupSender<A>,
        state: ActorState<A>,
    ) -> Self {
        let subs = if state.allow_subscribe() {
            Some(Default::default())
        } else {
            None
        };

        let tx_subs = if state.allow_broadcast() {
            Some(tx_subs)
        } else {
            None
        };

        Self {
            strong_count: RefCounter::new(AtomicUsize::new(1)),
            tx,
            tx_subs,
            subs,
            state,
        }
    }

    pub(crate) fn weak_sender(&self) -> WeakSender<ContextMessage<A>> {
        self.tx.downgrade()
    }

    fn send_timeout(
        &self,
        msg: ContextMessage<A>,
    ) -> impl Future<Output = Result<(), ActixSendError>> + '_ {
        self.tx.send_timeout(msg, self.state.timeout())
    }
}

impl<A> Address<A>
where
    A: Actor,
{
    /// Send a message to actor(s) and await for result.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn send<M>(
        &self,
        msg: M,
    ) -> Result<<M as MapResult<A::Result>>::Output, ActixSendError>
    where
        M: Into<A::Message> + MapResult<A::Result>,
    {
        let (tx, rx) = oneshot_channel();

        let msg = ContextMessage::Instant(InstantMessage::Static(Some(tx), msg.into()));

        self.send_timeout(msg).await?;

        let res = rx.await.map_err(|_| ActixSendError::Canceled)?;

        M::map(res)
    }

    /// Send a message to actor(s) and ignore the result.
    pub fn do_send(&self, msg: impl Into<A::Message>) {
        let msg = ContextMessage::Instant(InstantMessage::Static(None, msg.into()));
        let this = self.tx.clone();
        runtime::spawn(async move {
            let _ = this.send(msg).await;
        });
    }

    /// Send a message after a certain amount of delay.
    ///
    /// *. If `Address` is dropped we lose all pending messages that have not met the delay deadline.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn send_later(
        &self,
        msg: impl Into<A::Message>,
        delay: Duration,
    ) -> Result<(), ActixSendError> {
        let msg = ContextMessage::Delayed(DelayedMessage::Static(msg.into(), delay));
        self.send_timeout(msg).await?;
        Ok(())
    }

    /// Send a stream to actor(s) and return a new stream applied with `Handler::handle` method.
    ///
    /// *. Item of the stream must be one of actor's message types.
    /// (Or with trait Into<Actor::Message> impl)
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub fn send_stream<S, I, M>(&self, stream: S) -> ActorStream<A, S, I, M>
    where
        S: Stream<Item = I>,
        I: Into<M>,
        M: Into<A::Message> + MapResult<A::Result> + 'static,
    {
        ActorStream::new(stream, self.tx.clone())
    }

    /// Send a skip stream to actor(s) and return a new stream applied with `Handler::handle`
    /// method.
    ///
    /// Original stream would produce optional item and when None is produced ActorSkipStream would
    /// keep poll_next until the item of original stream become Some or goes into pending/finished
    /// state.
    ///
    /// *. `Some` variant of the original stream's Item must be one of actor's message types.
    /// (Or with trait Into<Actor::Message> impl)
    #[deprecated(
        note = "send_skip_stream will be removed in future. please consider use futures_util::stream::stream::skip_while instead"
    )]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub fn send_skip_stream<S, I, M>(&self, stream: S) -> ActorSkipStream<A, S, I, M>
    where
        S: Stream<Item = Option<I>>,
        I: Into<M>,
        M: Into<A::Message> + MapResult<A::Result> + 'static,
    {
        ActorSkipStream::new(stream, self.tx.clone())
    }

    /// Send a broadcast message to all actor instances that are alive for this address.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn broadcast<M>(
        &self,
        msg: M,
    ) -> Vec<Result<<M as MapResult<A::Result>>::Output, ActixSendError>>
    where
        M: Into<A::Message> + MapResult<A::Result> + Clone,
    {
        let tx_subs = match self.tx_subs.as_ref() {
            Some(group) => group,
            None => return vec![Err(ActixSendError::Broadcast)],
        };

        tx_subs
            .as_slice()
            .iter()
            .fold(FuturesUnordered::new(), |fut, sub| {
                let (tx, rx) = oneshot_channel();

                let msg =
                    ContextMessage::Instant(InstantMessage::Static(Some(tx), msg.clone().into()));

                let f = async move {
                    let f = sub.send(msg);
                    runtime::timeout(self.state.timeout(), f)
                        .await?
                        .map_err(|_| ActixSendError::Closed)?;
                    let rx = rx.await.map_err(|_| ActixSendError::Canceled)?;
                    M::map(rx)
                };

                fut.push(f);

                fut
            })
            .collect()
            .await
    }

    /// add an address to the subscribe list to current address.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn subscribe_with<AA, M>(&self, addr: &Address<AA>) -> Result<(), ActixSendError>
    where
        AA: Actor,
        M: Send + Into<AA::Message> + 'static,
    {
        let weak = addr.weak_sender();

        self.subs
            .as_ref()
            .ok_or(ActixSendError::Subscribe)?
            .push::<AA, M>(weak)
            .await;

        Ok(())
    }

    /// send message to all subscribers of this actor.
    ///
    /// *. It's important the message type can be handled correctly by the subscriber actors.
    ///  A typecast error would return if the message type can not be handled by certain subscriber
    /// actor.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn send_subscribe<M>(&self, msg: M) -> Vec<Result<(), ActixSendError>>
    where
        M: Clone + Send + 'static,
    {
        let subs = match self.subs.as_ref() {
            Some(group) => group,
            None => return vec![Err(ActixSendError::Subscribe)],
        };

        subs.lock()
            .await
            .iter()
            .fold(FuturesUnordered::new(), |fut, sub| {
                let msg = msg.clone();
                let timeout = self.state.timeout();

                let f = sub.send(AnyObjectContainer::pack(msg), timeout);

                fut.push(f);

                fut
            })
            .collect::<Vec<Option<Result<(), ActixSendError>>>>()
            .await
            .into_iter()
            .filter_map(|s| s)
            .collect()
    }

    /// Close one actor context for this address.
    ///
    /// Would a return a struct contains the closed context's state.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn close_one(&self) -> Result<ActorContextState, ActixSendError> {
        let (tx, rx) = oneshot_channel();
        let msg = ContextMessage::ManualShutDown(tx);
        self.send_timeout(msg).await?;
        Ok(rx.await.map_err(|_| ActixSendError::Canceled)?)
    }
}

macro_rules! address_run {
    ($($send:ident)*) => {
        impl<A> Address<A>
        where
            A: Actor,
        {
            /// Run a boxed future on actor(s).
            ///
            /// This function use dynamic dispatches to interact with actor.
            ///
            /// It gives you flexibility in exchange of some performance
            /// (Each `Address::run` would have two more heap allocation than `Address::send`)
            #[must_use = "futures do nothing unless you `.await` or poll them"]
            pub async fn run<F, R>(&self, f: F) -> Result<R, ActixSendError>
            where
                F: FnMut(&mut A) -> Pin<Box<dyn Future<Output = R> $( + $send)* + '_>> + Send + 'static,
                R: Send + 'static,
            {
                let (tx, rx) = oneshot_channel();

                let object = crate::object::FutureObject(f, PhantomData, std::sync::atomic::AtomicPtr::default()).pack();

                let msg = ContextMessage::Instant(InstantMessage::Dynamic(Some(tx), object));

                self.send_timeout(msg).await?;

                rx.await.map_err(|_| ActixSendError::Canceled)?.unpack::<R>().ok_or(ActixSendError::TypeCast)
            }

            /// Run a boxed future and ignore the result.
            pub fn do_run<F>(&self, f: F)
            where
                F: FnMut(&mut A) -> Pin<Box<dyn Future<Output = ()> $( + $send)* + '_>> + Send + 'static,
            {
                let object = crate::object::FutureObject(f, PhantomData, std::sync::atomic::AtomicPtr::default()).pack();
                let msg = ContextMessage::Instant(InstantMessage::Dynamic(None, object));

                let this = self.tx.clone();
                runtime::spawn(async move {
                    let _ = this.send(msg).await;
                });
            }

            /// Run a boxed future after a certain amount of delay.
            ///
            /// *. If `Address` is dropped we lose all pending boxed futures that have not met the delay deadline.
            #[must_use = "futures do nothing unless you `.await` or poll them"]
            pub async fn run_later<F>(&self, delay: Duration, f: F) -> Result<(), ActixSendError>
            where
                F: FnMut(&mut A) -> Pin<Box<dyn Future<Output = ()> $( + $send)* + '_>> + Send + 'static,
            {
                let object = crate::object::FutureObject(f, PhantomData, std::sync::atomic::AtomicPtr::default()).pack();

                let msg = ContextMessage::Delayed(DelayedMessage::Dynamic(object, delay));

                self.send_timeout(msg).await?;

                Ok(())
            }

            /// Register an interval future for actor(s). A set of actor(s) can have multiple interval futures registered.
            ///
            /// a `FutureHandler` would return that can be used to cancel it.
            ///
            /// *. dropping the `FutureHandler` would do nothing and the interval futures will be active until the address is dropped.
            #[must_use = "futures do nothing unless you `.await` or poll them"]
            pub async fn run_interval<F>(
                &self,
                dur: Duration,
                f: F,
            ) -> Result<FutureHandler<A>, ActixSendError>
            where
                F: FnMut(&mut A) -> Pin<Box<dyn Future<Output = ()> $( + $send)* + '_>> + Send + 'static,
            {
                let (tx, rx) = oneshot_channel();

                let object = crate::object::FutureObject(f, PhantomData, std::sync::atomic::AtomicPtr::default()).pack();

                let msg = ContextMessage::Interval(IntervalMessage::Register(tx, object, dur));

                self.send_timeout(msg).await?;

                Ok(rx.await.map_err(|_| ActixSendError::Canceled)?)
            }
        }
    };
}

#[cfg(not(any(
    feature = "actix-runtime",
    feature = "actix-runtime-mpsc",
    feature = "actix-runtime-local"
)))]
address_run!(Send);

#[cfg(any(
    feature = "actix-runtime",
    feature = "actix-runtime-mpsc",
    feature = "actix-runtime-local"
))]
address_run!();

pub struct WeakAddress<A>
where
    A: Actor,
{
    strong_count: RefCounter<AtomicUsize>,
    tx: WeakSender<ContextMessage<A>>,
    tx_subs: Option<WeakGroupSender<A>>,
    state: ActorState<A>,
}

impl<A> WeakAddress<A>
where
    A: Actor,
{
    pub fn upgrade(&self) -> Option<Address<A>> {
        self.tx.upgrade().map(|sender| {
            self.strong_count.fetch_add(1, Ordering::SeqCst);
            Address {
                strong_count: self.strong_count.clone(),
                tx: sender,
                tx_subs: self
                    .tx_subs
                    .as_ref()
                    .map(|sub| sub.upgrade().expect("Failed to upgrade WeakGroupSender")),
                subs: None,
                state: self.state.clone(),
            }
        })
    }
}

// a helper trait for map result of original messages.
// M here is auto generated ActorResult from #[actor_mod] macro.
pub trait MapResult<M>: Sized {
    type Output;
    fn map(msg: M) -> Result<Self::Output, ActixSendError>;
}
