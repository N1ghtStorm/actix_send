use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use futures::channel::oneshot::channel;
use parking_lot::Mutex;

use crate::actors::Actor;
use crate::context::ChannelMessage;
use crate::error::ActixSendError;
use crate::object::FutureResultObjectContainer;
use crate::util::{future_handle::FutureHandler, runtime};

// A channel sender for communicating with actor(s).
pub struct Address<A>
where
    A: Actor + 'static,
{
    tx: Sender<ChannelMessage<A>>,
    handlers: Arc<Mutex<Vec<FutureHandler<A>>>>,
    _a: PhantomData<A>,
}

impl<A> Address<A>
where
    A: Actor,
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
    A: Actor,
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

        M::map(res)
    }

    /// Send a message to actor and ignore the result.
    pub fn do_send(&self, msg: impl Into<A::Message>) {
        let msg = ChannelMessage::Instant(None, msg.into());
        let this = self.tx.clone();
        runtime::spawn(async move {
            let _ = this.send(msg).await;
        });
    }

    /// Send a message after a certain amount of delay.
    ///
    /// *. If address is dropped we lose all pending messages that have not met the delay deadline.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn send_later(
        &self,
        msg: impl Into<A::Message>,
        delay: Duration,
    ) -> Result<(), ActixSendError> {
        let msg = ChannelMessage::Delayed(msg.into(), delay);
        self.tx.send(msg).await?;
        Ok(())
    }

    /// Run a boxed future on actor.
    ///
    /// This function use dynamic dispatches to interact with actor.
    ///
    /// It gives you flexibility in exchange of some performance
    /// (Each `Address::run` would have two more heap allocation than `Address::send`)
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn run<F, R>(&self, f: F) -> Result<R, ActixSendError>
    where
        for<'a> F: Fn(&'a mut A) -> Pin<Box<dyn Future<Output = R> + Send + 'a>> + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = channel::<FutureResultObjectContainer>();

        let object = crate::object::FutureObject(f, PhantomData, PhantomData).pack();

        self.tx
            .send(ChannelMessage::InstantDynamic(Some(tx), object))
            .await?;

        let mut res: FutureResultObjectContainer = rx.await?;

        let r = res.unpack::<R>();

        r.ok_or(ActixSendError::TypeCast)
    }

    /// Run a boxed future and ignore the result.    
    pub fn do_run<F>(&self, f: F)
    where
        for<'a> F: Fn(&'a mut A) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send + 'static,
    {
        let object = crate::object::FutureObject(f, PhantomData, PhantomData).pack();
        let msg = ChannelMessage::InstantDynamic(None, object);

        let this = self.tx.clone();
        runtime::spawn(async move {
            let _ = this.send(msg).await;
        });
    }

    /// Run a boxed future after a certain amount of delay.  
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub async fn run_later<F>(&self, delay: Duration, f: F) -> Result<(), ActixSendError>
    where
        for<'a> F: Fn(&'a mut A) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send + 'static,
    {
        let object = crate::object::FutureObject(f, PhantomData, PhantomData).pack();

        self.tx
            .send(ChannelMessage::DelayedDynamic(object, delay))
            .await?;

        Ok(())
    }

    /// Register an interval future for actor. An actor can have multiple interval futures registered.
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
        for<'a> F: Fn(&'a mut A) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send + 'static,
    {
        let (tx, rx) = channel::<FutureHandler<A>>();

        let object = crate::object::FutureObject(f, PhantomData, PhantomData).pack();

        self.tx
            .send(ChannelMessage::Interval(tx, object, dur))
            .await?;

        Ok(rx.await?)
    }
}

// a helper trait for map result of original messages.
// M here is auto generated ActorResult from #[actor_mod] macro.
pub trait MapResult<M>: Sized {
    type Output;
    fn map(msg: M) -> Result<Self::Output, ActixSendError>;
}
