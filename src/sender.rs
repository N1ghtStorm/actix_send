use core::time::Duration;
#[cfg(feature = "actix-runtime-local")]
use std::rc::{Rc as Wrapper, Weak as WeakWrapper};
#[cfg(not(feature = "actix-runtime-local"))]
use std::sync::{Arc as Wrapper, Weak as WeakWrapper};

use crate::actor::Actor;
use crate::context::ContextMessage;
use crate::error::ActixSendError;
use crate::util::channel::Sender as AsyncChannelSender;

// A wrapper for crate::util::channel::Sender so we have a unified abstraction for different
// channels

pub struct Sender<M> {
    inner: Wrapper<AsyncChannelSender<M>>,
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
            inner: Wrapper::new(sender),
        }
    }
}

impl<M> Sender<M>
where
    M: 'static,
{
    pub(crate) fn downgrade(&self) -> WeakSender<M> {
        WeakSender {
            inner: Wrapper::downgrade(&self.inner),
        }
    }
}

#[cfg(not(feature = "actix-runtime-local"))]
impl<M> Sender<M>
where
    M: 'static,
{
    pub(crate) async fn send(&self, msg: M) -> Result<(), ActixSendError> {
        self.inner
            .send(msg)
            .await
            .map_err(|_| ActixSendError::Closed)
    }

    pub(crate) fn try_send(&self, msg: M) -> Result<(), ActixSendError> {
        self.inner.try_send(msg).map_err(|_| ActixSendError::Closed)
    }

    pub(crate) async fn send_timeout(&self, msg: M, dur: Duration) -> Result<(), ActixSendError> {
        let fut = self.inner.send(msg);
        crate::util::runtime::timeout(dur, fut)
            .await?
            .map_err(|_| ActixSendError::Closed)?;
        Ok(())
    }
}

#[cfg(feature = "actix-runtime-local")]
impl<M> Sender<M>
where
    M: 'static,
{
    pub(crate) async fn send(&self, msg: M) -> Result<(), ActixSendError> {
        self.inner.send(msg).map_err(|_| ActixSendError::Closed)
    }

    pub(crate) fn try_send(&self, msg: M) -> Result<(), ActixSendError> {
        self.inner.send(msg).map_err(|_| ActixSendError::Closed)
    }

    pub(crate) async fn send_timeout(&self, msg: M, dur: Duration) -> Result<(), ActixSendError> {
        drop(dur);
        self.send(msg).await
    }
}

pub struct WeakSender<M> {
    inner: WeakWrapper<AsyncChannelSender<M>>,
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
        WeakWrapper::upgrade(&self.inner).map(|inner| Sender { inner })
    }
}

// ToDo: for now there is no way to remove closed actor instance.
pub struct GroupSender<A>
where
    A: Actor,
{
    inner: Wrapper<Vec<Sender<ContextMessage<A>>>>,
}

impl<A> From<Vec<Sender<ContextMessage<A>>>> for GroupSender<A>
where
    A: Actor,
{
    fn from(sender: Vec<Sender<ContextMessage<A>>>) -> Self {
        Self {
            inner: Wrapper::new(sender),
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
            inner: Wrapper::downgrade(&self.inner),
        }
    }

    pub(crate) fn as_slice(&self) -> &[Sender<ContextMessage<A>>] {
        self.inner.as_slice()
    }
}

pub struct WeakGroupSender<A>
where
    A: Actor,
{
    inner: WeakWrapper<Vec<Sender<ContextMessage<A>>>>,
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
        WeakWrapper::upgrade(&self.inner).map(|inner| GroupSender { inner })
    }
}
