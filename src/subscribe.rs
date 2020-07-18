use core::any::Any;
use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::time::Duration;

use std::sync::Arc;
use std::thread::JoinHandle;

use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

use crate::actor::Actor;
use crate::context::ContextMessage;
use crate::error::ActixSendError;
use crate::sender::WeakSender;
use crate::util::runtime;

// subscribe hold a vector of trait objects which are boxed Subscriber that contains
// Sender<ContextMessage<Actor>> and an associate message type.
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

// we make Subscriber to trait object so they are not bound to Actor and Message Type.
pub(crate) trait SubscribeTrait {
    fn send(
        &self,
        // Input message type have to be boxed too.
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

            let msg = msg.inner.as_any_mut().downcast_mut::<Option<M>>()?.take()?;
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

trait MessageTrait {
    fn as_any_mut(&mut self) -> &mut dyn Any;
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

// impl Message trait for Option<MessageType>.
impl<M> MessageTrait for Option<M>
where
    M: Send + 'static,
{
    // cast self to any so we can downcast it to a concrete type.
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
