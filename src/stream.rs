use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use futures_util::stream::Stream;
use pin_project::pin_project;
use tokio::sync::oneshot::channel;

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
    #[allow(clippy::type_complexity)]
    pending_future: Option<
        Pin<Box<dyn Future<Output = Result<<I as MapResult<A::Result>>::Output, ActixSendError>>>>,
    >,
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
            pending_future: None,
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

        // If we have a pending_future it means we are waiting for the last result of stream item.
        if let Some(fut) = this.pending_future.as_mut() {
            return match fut.as_mut().poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(res) => {
                    *this.pending_future = None;
                    Poll::Ready(Some(res))
                }
            };
        }

        // poll and handle a new stream item.
        match this.stream.as_mut().poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(item)) => {
                // ToDo: we make box and clone sender with every stream item for now which is not optimal.
                let mut fut = Box::pin(send(this.tx.clone(), item));

                match fut.as_mut().poll(cx) {
                    Poll::Pending => {
                        *this.pending_future = Some(fut);
                        Poll::Pending
                    }
                    Poll::Ready(res) => Poll::Ready(Some(res)),
                }
            }
        }
    }
}

async fn send<A, I>(
    sender: Sender<ContextMessage<A>>,
    item: I,
) -> Result<<I as MapResult<A::Result>>::Output, ActixSendError>
where
    A: Actor + 'static,
    I: Into<A::Message> + MapResult<A::Result>,
{
    let (tx, rx) = channel::<A::Result>();

    let msg = ContextMessage::Instant(InstantMessage::Static(Some(tx), item.into()));
    sender.send(msg).await?;

    let res = rx.await?;
    I::map(res)
}
