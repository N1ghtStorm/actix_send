use core::any::Any;
use core::future::Future;
use core::pin::Pin;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

use crate::actor::Actor;
use crate::context::ContextMessage;
use crate::error::ActixSendError;
use crate::sender::WeakSender;
use crate::util::runtime;
use std::thread::JoinHandle;

// subscribe hold a vector of trait objects which are boxed WeakSender<ContextMessage<Actor>>
pub(crate) struct Subscribe {
    inner: Arc<AsyncMutex<Vec<Box<dyn SubscribeTrait + Send>>>>,
}

impl Default for Subscribe {
    fn default() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

impl Clone for Subscribe {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Subscribe {
    pub(crate) async fn push<A, M>(&self, sender: WeakSender<ContextMessage<A>>)
    where
        A: Actor + 'static,
        M: Send + Into<A::Message> + 'static,
    {
        self.lock().await.push(Box::new(Subscriber {
            sender,
            _message: PhantomData::<JoinHandle<M>>,
        }));
    }

    pub(crate) async fn lock(&self) -> AsyncMutexGuard<'_, Vec<Box<dyn SubscribeTrait + Send>>> {
        self.inner.lock().await
    }
}

struct Subscriber<A, M>
where
    A: Actor + 'static,
    M: Send + 'static,
{
    sender: WeakSender<ContextMessage<A>>,
    _message: PhantomData<JoinHandle<M>>,
}

// We have to make our message type a trait object when we want to send a message to multiple actors
// in a single function call. Because ContextMessage<A> is bound to A which is a single actor type.
trait MessageTrait {
    fn as_mut_any(&mut self) -> &mut dyn Any;
}

// a container for message trait object
pub(crate) struct MessageContainer {
    inner: Box<dyn MessageTrait + Send>,
}

// pack message type into trait object
impl MessageContainer {
    pub(crate) fn pack<M>(msg: M) -> Self
    where
        M: Send + 'static,
    {
        Self {
            inner: Box::new(Some(msg)),
        }
    }
}

// impl Message trait for Option<ContextMessageWithSender> as we want to take it out after downcast.
impl<M> MessageTrait for Option<M>
where
    M: Send + 'static,
{
    // cast self to any so we can downcast it to a concrete type.
    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}

// we make WeakSender and Message type to trait objects so they can not bound to A: Actor.
pub(crate) trait SubscribeTrait {
    fn send(
        &self,
        msg: MessageContainer,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Option<Result<(), ActixSendError>>> + Send + '_>>;
}

impl<A, M> SubscribeTrait for Subscriber<A, M>
where
    A: Actor + 'static,
    M: Send + Into<A::Message> + 'static,
{
    fn send(
        &self,
        mut msg: MessageContainer,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Option<Result<(), ActixSendError>>> + Send + '_>> {
        Box::pin(async move {
            // We downcast message trait object to the Message type of WeakSender.

            let msg = msg.inner.as_mut_any().downcast_mut::<Option<M>>()?.take()?;
            let res = self._send(msg, timeout).await;
            Some(res)
        })
    }
}

impl<A, M> Subscriber<A, M>
where
    A: Actor + 'static,
    M: Send + Into<A::Message> + 'static,
{
    async fn _send(&self, msg: M, timeout: Duration) -> Result<(), ActixSendError> {
        let sender = self.sender.upgrade().ok_or(ActixSendError::Closed)?;
        let f = sender.send(ContextMessage::Instant(None, msg.into()));

        runtime::timeout(timeout, f).await??;

        Ok(())
    }
}
