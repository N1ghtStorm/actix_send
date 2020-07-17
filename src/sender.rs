use core::time::Duration;
use std::sync::{Arc, Weak};

use async_channel::{SendError, Sender as AsyncChannelSender};

use crate::actor::Actor;
use crate::context::ContextMessage;
use crate::error::ActixSendError;
use crate::util::runtime;

// A wrapper for async_channel::sender.
// ToDo: remove this when we have a weak sender.

pub struct Sender<M> {
    inner: Arc<AsyncChannelSender<M>>,
}

impl<M> Clone for Sender<M> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<M> From<AsyncChannelSender<M>> for Sender<M> {
    fn from(sender: AsyncChannelSender<M>) -> Self {
        Self {
            inner: Arc::new(sender),
        }
    }
}

impl<M> Sender<M>
where
    M: Send,
{
    pub(crate) fn downgrade(&self) -> WeakSender<M> {
        WeakSender {
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub(crate) async fn send(&self, msg: M) -> Result<(), SendError<M>> {
        self.inner.send(msg).await
    }

    pub(crate) async fn send_timeout(&self, msg: M, dur: Duration) -> Result<(), ActixSendError> {
        let fut = self.inner.send(msg);
        runtime::timeout(dur, fut).await??;
        Ok(())
    }
}

pub struct WeakSender<M> {
    inner: Weak<async_channel::Sender<M>>,
}

impl<M> Clone for WeakSender<M> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<M> WeakSender<M> {
    pub(crate) fn upgrade(&self) -> Option<Sender<M>> {
        Weak::upgrade(&self.inner).map(|inner| Sender { inner })
    }
}

// ToDo: for now there is no way to remove closed actor instance.
pub struct GroupSender<A>
where
    A: Actor,
{
    inner: Arc<Vec<AsyncChannelSender<ContextMessage<A>>>>,
}

impl<A> From<Vec<AsyncChannelSender<ContextMessage<A>>>> for GroupSender<A>
where
    A: Actor,
{
    fn from(sender: Vec<AsyncChannelSender<ContextMessage<A>>>) -> Self {
        Self {
            inner: Arc::new(sender),
        }
    }
}

impl<A> Clone for GroupSender<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<A> GroupSender<A>
where
    A: Actor,
{
    pub(crate) fn downgrade(&self) -> WeakGroupSender<A> {
        WeakGroupSender {
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub(crate) fn as_slice(&self) -> &[AsyncChannelSender<ContextMessage<A>>] {
        self.inner.as_slice()
    }
}

pub struct WeakGroupSender<A>
where
    A: Actor,
{
    inner: Weak<Vec<AsyncChannelSender<ContextMessage<A>>>>,
}

impl<A> Clone for WeakGroupSender<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<A> WeakGroupSender<A>
where
    A: Actor,
{
    pub(crate) fn upgrade(&self) -> Option<GroupSender<A>> {
        Weak::upgrade(&self.inner).map(|inner| GroupSender { inner })
    }
}
