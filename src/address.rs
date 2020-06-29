use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use futures::channel::oneshot::channel;
use parking_lot::Mutex;

use crate::actors::Actor;
use crate::context::ChannelMessage;
use crate::error::ActixSendError;
use crate::util::{future_handle::FutureHandler, runtime};

// A channel sender for communicating with actor(s).
pub struct Address<A>
where
    A: Actor + 'static,
    A::Message: Send,
    A::Result: Send,
{
    tx: Sender<ChannelMessage<A>>,
    handlers: Arc<Mutex<Vec<FutureHandler<A>>>>,
    _a: PhantomData<A>,
}

impl<A> Address<A>
where
    A: Actor + 'static,
    A::Message: Send,
    A::Result: Send,
{
    pub(crate) fn new(tx: Sender<ChannelMessage<A>>, handlers: Vec<FutureHandler<A>>) -> Self {
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
    A::Message: Send,
    A::Result: Send,
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
    A::Message: Send,
    A::Result: Send,
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
    A::Message: Send,
    A::Result: Send,
{
    /// Send a message to actor and await for result.
    ///
    /// Message will be returned in `ActixSendError::Closed(Message)` if the actor is already closed.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn send<M>(
        &self,
        msg: M,
    ) -> Result<<M as MapResult<A::Result>>::Output, ActixSendError>
    where
        M: Into<A::Message> + MapResult<A::Result>,
    {
        let (tx, rx) = channel::<A::Result>();

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

    /// Run a message after a certain amount of delay.
    pub fn run_later(&self, msg: impl Into<A::Message> + Send + 'static, delay: Duration) {
        let msg = ChannelMessage::Delayed(msg.into(), delay);
        self._do_send(msg);
    }

    /// Register an interval future for actor. An actor can have multiple interval futures registered.
    ///
    /// a `FutureHandler` would return that can be used to cancel it.
    ///
    /// *. For now dropping the `FutureHandler` would do nothing and the interval future would last as long as the runtime.
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
