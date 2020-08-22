use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};

use futures_util::stream::Stream;
use pin_project::pin_project;

use crate::actor::Actor;
use crate::address::MapResult;
use crate::context::{ContextMessage, InstantMessage};
use crate::error::ActixSendError;
use crate::sender::Sender;
use crate::util::channel::{oneshot_channel, OneShotReceiver};

#[pin_project]
pub struct ActorStream<A, S, I, M>
where
    A: Actor,
    S: Stream<Item = I>,
    I: Into<M>,
    M: Into<A::Message> + MapResult<A::Result>,
{
    #[pin]
    stream: S,
    tx: Sender<ContextMessage<A>>,
    rx_one: Option<OneShotReceiver<A::Result>>,
    _m: PhantomData<M>,
}

impl<A, S, I, M> ActorStream<A, S, I, M>
where
    A: Actor,
    S: Stream<Item = I>,
    I: Into<M>,
    M: Into<A::Message> + MapResult<A::Result>,
{
    pub(crate) fn new(stream: S, tx: Sender<ContextMessage<A>>) -> Self {
        Self {
            stream,
            tx,
            rx_one: None,
            _m: PhantomData,
        }
    }
}

impl<A, S, I, M> Stream for ActorStream<A, S, I, M>
where
    A: Actor + 'static,
    S: Stream<Item = I>,
    I: Into<M>,
    M: Into<A::Message> + MapResult<A::Result>,
{
    type Item = Result<<M as MapResult<A::Result>>::Output, ActixSendError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        // if we have a one shot receiver then we are waiting for the last message result.
        if let Some(ref mut rx) = this.rx_one.as_mut() {
            return match Pin::new(rx).poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(res) => {
                    *this.rx_one = None;
                    match res {
                        Ok(res) => Poll::Ready(Some(M::map(res))),
                        Err(_) => Poll::Ready(Some(Err(ActixSendError::Canceled))),
                    }
                }
            };
        }

        // poll and handle a new stream item.
        match this.stream.as_mut().poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(item)) => {
                let (tx, mut rx) = oneshot_channel();
                let msg =
                    ContextMessage::Instant(InstantMessage::Static(Some(tx), item.into().into()));

                if let Err(e) = this.tx.try_send(msg) {
                    return Poll::Ready(Some(Err(e)));
                }

                match Pin::new(&mut rx).poll(cx) {
                    Poll::Pending => {
                        *this.rx_one = Some(rx);
                        Poll::Pending
                    }
                    Poll::Ready(res) => match res {
                        Ok(res) => Poll::Ready(Some(M::map(res))),
                        Err(_) => Poll::Ready(Some(Err(ActixSendError::Canceled))),
                    },
                }
            }
        }
    }
}

#[pin_project]
pub struct ActorSkipStream<A, S, I, M>
where
    A: Actor,
    S: Stream<Item = Option<I>>,
    I: Into<M>,
    M: Into<A::Message> + MapResult<A::Result>,
{
    #[pin]
    stream: S,
    tx: Sender<ContextMessage<A>>,
    rx_one: Option<OneShotReceiver<A::Result>>,
    _m: PhantomData<M>,
}

impl<A, S, I, M> ActorSkipStream<A, S, I, M>
where
    A: Actor,
    S: Stream<Item = Option<I>>,
    I: Into<M>,
    M: Into<A::Message> + MapResult<A::Result>,
{
    pub(crate) fn new(stream: S, tx: Sender<ContextMessage<A>>) -> Self {
        Self {
            stream,
            tx,
            rx_one: None,
            _m: PhantomData,
        }
    }
}

impl<A, S, I, M> Stream for ActorSkipStream<A, S, I, M>
where
    A: Actor + 'static,
    S: Stream<Item = Option<I>>,
    I: Into<M>,
    M: Into<A::Message> + MapResult<A::Result>,
{
    type Item = Result<<M as MapResult<A::Result>>::Output, ActixSendError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        // if we have a one shot receiver then we are waiting for the last message result.
        if let Some(ref mut rx) = this.rx_one.as_mut() {
            return match Pin::new(rx).poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(res) => {
                    *this.rx_one = None;
                    match res {
                        Ok(res) => Poll::Ready(Some(M::map(res))),
                        Err(_) => Poll::Ready(Some(Err(ActixSendError::Canceled))),
                    }
                }
            };
        }

        // poll and handle a new stream item.
        loop {
            match this.stream.as_mut().poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Ready(Some(None)) => continue,
                Poll::Ready(Some(Some(item))) => {
                    let (tx, mut rx) = oneshot_channel::<<A as Actor>::Result>();
                    let msg = ContextMessage::Instant(InstantMessage::Static(
                        Some(tx),
                        item.into().into(),
                    ));

                    if let Err(e) = this.tx.try_send(msg) {
                        return Poll::Ready(Some(Err(e)));
                    }

                    return match Pin::new(&mut rx).poll(cx) {
                        Poll::Pending => {
                            *this.rx_one = Some(rx);
                            Poll::Pending
                        }
                        Poll::Ready(res) => match res {
                            Ok(res) => Poll::Ready(Some(M::map(res))),
                            Err(_) => Poll::Ready(Some(Err(ActixSendError::Canceled))),
                        },
                    };
                }
            }
        }
    }
}
