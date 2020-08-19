use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use futures_util::stream::Stream;
use pin_project::pin_project;
use tokio::sync::oneshot::{channel, Receiver};

use crate::actor::Actor;
use crate::address::MapResult;
use crate::context::{ContextMessage, InstantMessage};
use crate::error::ActixSendError;
use crate::sender::Sender;

#[pin_project]
pub struct ActorStream<A, S, I>
where
    A: Actor,
    S: Stream<Item = I>,
    I: Into<A::Message> + MapResult<A::Result>,
{
    #[pin]
    stream: S,
    tx: Sender<ContextMessage<A>>,
    rx_one: Option<Receiver<A::Result>>,
}

impl<A, S, I> ActorStream<A, S, I>
where
    A: Actor,
    S: Stream<Item = I>,
    I: Into<A::Message> + MapResult<A::Result>,
{
    pub(crate) fn new(stream: S, tx: Sender<ContextMessage<A>>) -> Self {
        Self {
            stream,
            tx,
            rx_one: None,
        }
    }
}

impl<A, S, I> Stream for ActorStream<A, S, I>
where
    A: Actor + 'static,
    S: Stream<Item = I>,
    I: Into<A::Message> + MapResult<A::Result> + 'static,
{
    type Item = Result<<I as MapResult<A::Result>>::Output, ActixSendError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        // if we have a one shot receiver then we are waiting for the last message result.
        if let Some(ref mut rx) = this.rx_one.as_mut() {
            return match Pin::new(rx).poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(res) => {
                    *this.rx_one = None;
                    match res {
                        Ok(res) => Poll::Ready(Some(I::map(res))),
                        Err(e) => Poll::Ready(Some(Err(e.into()))),
                    }
                }
            };
        }

        // poll and handle a new stream item.
        match this.stream.as_mut().poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(item)) => {
                let (tx, mut rx) = channel::<A::Result>();
                let msg = ContextMessage::Instant(InstantMessage::Static(Some(tx), item.into()));

                if let Err(e) = this.tx.try_send(msg) {
                    return Poll::Ready(Some(Err(e.into())));
                }

                match Pin::new(&mut rx).poll(cx) {
                    Poll::Pending => {
                        *this.rx_one = Some(rx);
                        Poll::Pending
                    }
                    Poll::Ready(res) => match res {
                        Ok(res) => Poll::Ready(Some(I::map(res))),
                        Err(e) => Poll::Ready(Some(Err(e.into()))),
                    },
                }
            }
        }
    }
}
